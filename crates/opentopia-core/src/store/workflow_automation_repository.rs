use super::{collect_rows, deserialize_json_column, FlowOperationsStoreError, SqliteSessionStore};
use crate::workflow_automation::{
    FlowCaseV1, WorkflowDeliveryReceiptV1, WorkflowDeliveryStatusV1, WorkflowEvaluationV1,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

impl SqliteSessionStore {
    pub fn insert_flow_case(&self, case: &FlowCaseV1) -> anyhow::Result<FlowCaseV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO flow_cases (
                id, flow_id, trigger_id, idempotency_key, flow_revision_id,
                flow_run_id, status, input_hash, document_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                case.id.to_string(),
                &case.flow_id,
                case.trigger_id.to_string(),
                &case.idempotency_key,
                case.flow_revision_id.to_string(),
                case.flow_run_id.map(|id| id.to_string()),
                case.status.as_str(),
                &case.input_hash,
                serde_json::to_string(case)?,
                case.created_at.to_rfc3339(),
                case.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(case.clone())
    }

    pub fn get_flow_case(
        &self,
        flow_id: &str,
        idempotency_key: &str,
    ) -> anyhow::Result<Option<FlowCaseV1>> {
        let conn = self.read_connection();
        Ok(conn
            .query_row(
                "SELECT document_json FROM flow_cases WHERE flow_id = ?1 AND idempotency_key = ?2",
                params![flow_id, idempotency_key],
                deserialize_json_column::<FlowCaseV1>,
            )
            .optional()?)
    }

    pub fn get_flow_case_by_id(&self, case_id: Uuid) -> anyhow::Result<Option<FlowCaseV1>> {
        let conn = self.read_connection();
        Ok(conn
            .query_row(
                "SELECT document_json FROM flow_cases WHERE id = ?1",
                params![case_id.to_string()],
                deserialize_json_column::<FlowCaseV1>,
            )
            .optional()?)
    }

    pub fn list_flow_cases(&self, flow_id: Option<&str>) -> anyhow::Result<Vec<FlowCaseV1>> {
        let conn = self.read_connection();
        let mut statement = match flow_id {
            Some(_) => conn.prepare("SELECT document_json FROM flow_cases WHERE flow_id = ?1 ORDER BY updated_at DESC LIMIT 500")?,
            None => conn.prepare("SELECT document_json FROM flow_cases ORDER BY updated_at DESC LIMIT 500")?,
        };
        let rows = match flow_id {
            Some(id) => statement.query_map(params![id], deserialize_json_column::<FlowCaseV1>)?,
            None => statement.query_map([], deserialize_json_column::<FlowCaseV1>)?,
        };
        collect_rows(rows)
    }

    pub fn count_recent_flow_cases(
        &self,
        trigger_id: Uuid,
        since: DateTime<Utc>,
    ) -> anyhow::Result<u32> {
        let conn = self.read_connection();
        let count = conn.query_row(
            "SELECT COUNT(*) FROM flow_cases WHERE trigger_id = ?1 AND created_at >= ?2",
            params![trigger_id.to_string(), since.to_rfc3339()],
            |row| row.get::<_, u32>(0),
        )?;
        Ok(count)
    }

    pub fn update_flow_case(&self, case: &FlowCaseV1) -> anyhow::Result<FlowCaseV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let changed = conn.execute(
            r#"
            UPDATE flow_cases
            SET flow_run_id = ?2, status = ?3, document_json = ?4, updated_at = ?5
            WHERE id = ?1
            "#,
            params![
                case.id.to_string(),
                case.flow_run_id.map(|id| id.to_string()),
                case.status.as_str(),
                serde_json::to_string(case)?,
                case.updated_at.to_rfc3339(),
            ],
        )?;
        anyhow::ensure!(changed == 1, "Flow case not found");
        Ok(case.clone())
    }

    pub fn insert_workflow_delivery_receipt(
        &self,
        receipt: &WorkflowDeliveryReceiptV1,
    ) -> anyhow::Result<WorkflowDeliveryReceiptV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO flow_delivery_receipts (
                id, revision, run_id, flow_revision_id, output_kind, status,
                document_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                receipt.id.to_string(),
                i64::from(receipt.revision),
                receipt.run_id.to_string(),
                receipt.flow_revision_id.to_string(),
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
                "SELECT document_json FROM flow_delivery_receipts WHERE id = ?1",
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
                "SELECT document_json FROM flow_delivery_receipts WHERE run_id = ?1",
                params![run_id.to_string()],
                deserialize_json_column::<WorkflowDeliveryReceiptV1>,
            )
            .optional()?)
    }

    pub fn list_workflow_delivery_receipts(
        &self,
        flow_revision_id: Option<Uuid>,
        status: Option<WorkflowDeliveryStatusV1>,
    ) -> anyhow::Result<Vec<WorkflowDeliveryReceiptV1>> {
        let conn = self.read_connection();
        let (sql, values): (&str, Vec<String>) = match (flow_revision_id, status) {
            (Some(revision_id), Some(status)) => (
                "SELECT document_json FROM flow_delivery_receipts WHERE flow_revision_id = ?1 AND status = ?2 ORDER BY updated_at DESC LIMIT 500",
                vec![revision_id.to_string(), status.as_str().to_string()],
            ),
            (Some(revision_id), None) => (
                "SELECT document_json FROM flow_delivery_receipts WHERE flow_revision_id = ?1 ORDER BY updated_at DESC LIMIT 500",
                vec![revision_id.to_string()],
            ),
            (None, Some(status)) => (
                "SELECT document_json FROM flow_delivery_receipts WHERE status = ?1 ORDER BY updated_at DESC LIMIT 500",
                vec![status.as_str().to_string()],
            ),
            (None, None) => (
                "SELECT document_json FROM flow_delivery_receipts ORDER BY updated_at DESC LIMIT 500",
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
            UPDATE flow_delivery_receipts
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
                    "SELECT revision FROM flow_delivery_receipts WHERE id = ?1",
                    params![receipt.id.to_string()],
                    |row| row.get::<_, u32>(0),
                )
                .optional()?;
            return Err(match current {
                Some(revision) => {
                    FlowOperationsStoreError::DeliveryReceiptRevisionConflict(revision).into()
                }
                None => FlowOperationsStoreError::DeliveryReceiptNotFound(receipt.id).into(),
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
            INSERT INTO flow_evaluations (
                id, run_id, flow_revision_id, evaluator, passed, score,
                document_json, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                evaluation.id.to_string(),
                evaluation.run_id.to_string(),
                evaluation.flow_revision_id.to_string(),
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
                "SELECT document_json FROM flow_evaluations WHERE run_id = ?1 AND evaluator = ?2",
                params![run_id.to_string(), evaluator],
                deserialize_json_column::<WorkflowEvaluationV1>,
            )
            .optional()?)
    }

    pub fn list_workflow_evaluations(
        &self,
        flow_revision_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<WorkflowEvaluationV1>> {
        let conn = self.read_connection();
        let mut statement = match flow_revision_id {
            Some(_) => conn.prepare("SELECT document_json FROM flow_evaluations WHERE flow_revision_id = ?1 ORDER BY created_at DESC LIMIT 1000")?,
            None => conn.prepare("SELECT document_json FROM flow_evaluations ORDER BY created_at DESC LIMIT 1000")?,
        };
        let rows = match flow_revision_id {
            Some(id) => statement.query_map(
                params![id.to_string()],
                deserialize_json_column::<WorkflowEvaluationV1>,
            )?,
            None => statement.query_map([], deserialize_json_column::<WorkflowEvaluationV1>)?,
        };
        collect_rows(rows)
    }
}
