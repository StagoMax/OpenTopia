use crate::connection::{
    ConnectionCapabilityRevisionV1, ConnectionStatusV1, ConnectionV1, IntegrationDefinitionV1,
};
use crate::effect_journal::{
    valid_effect_transition, validate_effect_intent, EffectIntent, EffectJournalError,
    EffectJournalRecord, EffectStatus,
};
use crate::enterprise::{AgentTemplateStatusV1, AgentTemplateVersionV1};
use crate::flow::{
    definition_from_draft, FlowDefinitionV1, FlowDraftStatusV1, FlowDraftV1, FlowTrialV1,
};
use crate::flow_runtime::FlowRunV1;
use crate::human_task::{HumanTaskStatusV1, HumanTaskV1};
use crate::model::{
    AgentEvent, AgentEventPayload, Approval, ApprovalStatus, Artifact, ArtifactMetadata,
    ArtifactStorage, ExperienceMode, GoalRecord, GoalSnapshot, GoalStatus, Message, Project,
    TerminalCommandHistory, Thread, ThreadModelSelection, TurnChangeSet, TurnRecord, TurnStatus,
    UserInputRecord, UserInputRequest, UserInputResponse, UserInputStatus,
};
use crate::work_form::{WorkForm, WorkScope};
use crate::workflow::{WorkflowDeploymentStatusV1, WorkflowDeploymentV1};
use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicUsize, Mutex};
use uuid::Uuid;

mod sqlite_codec;
use sqlite_codec::*;
mod sqlite_rows;
use sqlite_rows::*;
mod agent_flow_repository;
mod agent_repository;
mod connection_repository;
mod flow_runtime_repository;
mod goal_event_repository;
mod legacy_schema;
mod project_repository;
mod settings_mcp_repository;
mod sqlite_runtime;
use agent_flow_repository::query_flow_draft;
use flow_runtime_repository::{
    insert_human_task_conn, update_flow_run_conn, update_human_task_conn,
};
use goal_event_repository::{
    conversation_payload_json, effect_select_sql, load_goal_snapshot, query_effect,
    query_effect_by_idempotency_key, query_work_form, query_work_form_for_scope,
    upsert_work_form_conn, valid_goal_transition, validate_goal_definition_list,
};
pub use project_repository::normalize_workspace_key;
use project_repository::{
    ensure_workspace_available, insert_project, insert_thread, project_workspace_values,
    query_project, query_project_by_workspace_key, query_thread, touch_thread,
    validated_project_name, validated_workspace_key,
};

mod session_store;
pub use session_store::SessionStore;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("project name cannot be empty")]
    EmptyProjectName,
    #[error("thread title cannot be empty")]
    EmptyThreadTitle,
    #[error("workspace root cannot be empty")]
    EmptyWorkspaceRoot,
    #[error("a project already exists for workspace: {0}")]
    DuplicateWorkspace(String),
    #[error("project not found: {0}")]
    ProjectNotFound(Uuid),
    #[error("project has no workspace root: {0}")]
    ProjectHasNoWorkspace(Uuid),
    #[error("project workspace root cannot be cleared while it owns threads: {0}")]
    ProjectWorkspaceInUse(Uuid),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AgentTemplateStoreError {
    #[error("Agent template not found: {0}")]
    TemplateNotFound(String),
    #[error("Agent template version not found: {template_id}@{version}")]
    VersionNotFound { template_id: String, version: u32 },
    #[error("Agent template is archived: {0}")]
    TemplateArchived(String),
    #[error("Agent template owner cannot change across versions")]
    OwnerMismatch,
    #[error("only draft Agent template versions can be deleted")]
    PublishedVersionIsImmutable,
    #[error("a newer Agent template version is already published")]
    StaleVersion,
    #[error("Agent template version is already referenced by an instance")]
    VersionInUse,
    #[error("Agent instance not found: {0}")]
    InstanceNotFound(Uuid),
    #[error("Agent state revision conflict; current revision is {0}")]
    StateRevisionConflict(u64),
    #[error("only active root Agent instances can be bound to a thread")]
    InvalidThreadBinding,
    #[error("Agent instance belongs to a different thread")]
    InstanceThreadMismatch,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FlowStoreError {
    #[error("Flow draft not found: {0}")]
    DraftNotFound(Uuid),
    #[error("Flow draft revision conflict; current revision is {0}")]
    RevisionConflict(u32),
    #[error("Flow draft cannot be published until validation passes")]
    ValidationRequired,
    #[error("Flow draft cannot be published without a passed simulation for its current revision")]
    PassedTrialRequired,
    #[error("high-risk Flow publication requires an independent approver")]
    IndependentApproverRequired,
    #[error("Flow run not found: {0}")]
    RunNotFound(Uuid),
    #[error("Flow run revision conflict; current revision is {0}")]
    RunRevisionConflict(u32),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorkflowDeploymentStoreError {
    #[error("Workflow deployment not found: {0}")]
    NotFound(Uuid),
    #[error("Workflow deployment revision conflict; current revision is {0}")]
    RevisionConflict(u32),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HumanTaskStoreError {
    #[error("Human task not found: {0}")]
    NotFound(Uuid),
    #[error("Human task revision conflict; current revision is {0}")]
    RevisionConflict(u32),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConnectionStoreError {
    #[error("Integration definition not found: {0}")]
    IntegrationDefinitionNotFound(Uuid),
    #[error("Integration definition revision conflict; current revision is {0}")]
    IntegrationDefinitionRevisionConflict(u32),
    #[error("Integration definition key already exists: {0}")]
    DuplicateIntegrationKey(String),
    #[error("Connection not found: {0}")]
    ConnectionNotFound(Uuid),
    #[error("Connection revision conflict; current revision is {0}")]
    ConnectionRevisionConflict(u32),
    #[error("MCP server runtime is already bound to a different Connection: {0}")]
    McpRuntimeAlreadyBound(Uuid),
    #[error("Connection capability revision not found: {connection_id}@{revision}")]
    CapabilityRevisionNotFound { connection_id: Uuid, revision: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextBudget {
    pub total_tokens: usize,
    pub used_tokens: usize,
    pub message_count: usize,
    pub estimated_usage: usize,
}

mod provider_conversation_state;
pub use provider_conversation_state::{ProviderContextStateKind, ProviderConversationState};

#[derive(Debug)]
pub struct SqliteSessionStore {
    conn: Mutex<Connection>,
    read_connections: Vec<Mutex<Connection>>,
    next_read_connection: AtomicUsize,
}

impl SessionStore for SqliteSessionStore {
    fn effective_plugin_settings(
        &self,
        plugin_id: &str,
        workspace_root: &Path,
        thread_id: Uuid,
    ) -> anyhow::Result<Value> {
        SqliteSessionStore::effective_plugin_settings(self, plugin_id, workspace_root, thread_id)
    }

    fn get_published_agent_template_version(
        &self,
        template_id: &str,
        version: u32,
    ) -> anyhow::Result<Option<AgentTemplateVersionV1>> {
        let template = SqliteSessionStore::get_agent_template_version(self, template_id, version)?;
        Ok(template.filter(|template| template.status == AgentTemplateStatusV1::Published))
    }

    fn insert_integration_definition(
        &self,
        definition: &IntegrationDefinitionV1,
    ) -> anyhow::Result<IntegrationDefinitionV1> {
        SqliteSessionStore::insert_integration_definition(self, definition)
    }

    fn get_integration_definition(
        &self,
        definition_id: Uuid,
    ) -> anyhow::Result<Option<IntegrationDefinitionV1>> {
        SqliteSessionStore::get_integration_definition(self, definition_id)
    }

    fn list_integration_definitions(&self) -> anyhow::Result<Vec<IntegrationDefinitionV1>> {
        SqliteSessionStore::list_integration_definitions(self)
    }

    fn update_integration_definition(
        &self,
        definition: &IntegrationDefinitionV1,
        expected_revision: u32,
    ) -> anyhow::Result<IntegrationDefinitionV1> {
        SqliteSessionStore::update_integration_definition(self, definition, expected_revision)
    }

    fn delete_integration_definition(&self, definition_id: Uuid) -> anyhow::Result<bool> {
        SqliteSessionStore::delete_integration_definition(self, definition_id)
    }

    fn insert_connection(&self, connection: &ConnectionV1) -> anyhow::Result<ConnectionV1> {
        SqliteSessionStore::insert_connection(self, connection)
    }

    fn get_connection(&self, connection_id: Uuid) -> anyhow::Result<Option<ConnectionV1>> {
        SqliteSessionStore::get_connection(self, connection_id)
    }

    fn list_connections(
        &self,
        integration_definition_id: Option<Uuid>,
        status: Option<ConnectionStatusV1>,
    ) -> anyhow::Result<Vec<ConnectionV1>> {
        SqliteSessionStore::list_connections(self, integration_definition_id, status)
    }

    fn update_connection(
        &self,
        connection: &ConnectionV1,
        expected_revision: u32,
    ) -> anyhow::Result<ConnectionV1> {
        SqliteSessionStore::update_connection(self, connection, expected_revision)
    }

    fn list_connection_capability_revisions(
        &self,
        connection_id: Uuid,
    ) -> anyhow::Result<Vec<ConnectionCapabilityRevisionV1>> {
        SqliteSessionStore::list_connection_capability_revisions(self, connection_id)
    }

    fn get_connection_capability_revision(
        &self,
        connection_id: Uuid,
        revision: u32,
    ) -> anyhow::Result<Option<ConnectionCapabilityRevisionV1>> {
        SqliteSessionStore::get_connection_capability_revision(self, connection_id, revision)
    }

    fn publish_connection_capability_revision(
        &self,
        connection: &ConnectionV1,
        expected_connection_revision: u32,
        capability_revision: &ConnectionCapabilityRevisionV1,
    ) -> anyhow::Result<(ConnectionV1, ConnectionCapabilityRevisionV1)> {
        SqliteSessionStore::publish_connection_capability_revision(
            self,
            connection,
            expected_connection_revision,
            capability_revision,
        )
    }

    fn create_flow_draft(&self, draft: &FlowDraftV1) -> anyhow::Result<FlowDraftV1> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO flow_drafts (
                id, thread_id, flow_id, revision, status, document_json,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                draft.id.to_string(),
                draft.thread_id.to_string(),
                &draft.spec.flow_id,
                i64::from(draft.revision),
                draft.status.as_str(),
                serde_json::to_string(draft)?,
                draft.created_at.to_rfc3339(),
                draft.updated_at.to_rfc3339(),
            ],
        )?;
        tx.execute(
            r#"
            INSERT INTO thread_flow_drafts (thread_id, draft_id, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(thread_id) DO UPDATE SET
                draft_id = excluded.draft_id,
                updated_at = excluded.updated_at
            "#,
            params![
                draft.thread_id.to_string(),
                draft.id.to_string(),
                draft.updated_at.to_rfc3339(),
            ],
        )?;
        tx.commit()?;
        Ok(draft.clone())
    }

    fn get_flow_draft(&self, draft_id: Uuid) -> anyhow::Result<Option<FlowDraftV1>> {
        let conn = self.read_connection();
        query_flow_draft(&conn, draft_id)
    }

    fn list_flow_drafts(&self, thread_id: Option<Uuid>) -> anyhow::Result<Vec<FlowDraftV1>> {
        let conn = self.read_connection();
        let mut stmt = conn.prepare(
            r#"
            SELECT document_json FROM flow_drafts
            WHERE (?1 IS NULL OR thread_id = ?1)
            ORDER BY updated_at DESC
            "#,
        )?;
        let thread_id = thread_id.map(|value| value.to_string());
        let rows = stmt.query_map(params![thread_id], deserialize_json_column::<FlowDraftV1>)?;
        collect_rows(rows)
    }

    fn update_flow_draft(
        &self,
        draft: &FlowDraftV1,
        expected_revision: u32,
    ) -> anyhow::Result<FlowDraftV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let changed = conn.execute(
            r#"
            UPDATE flow_drafts
            SET flow_id = ?2, revision = ?3, status = ?4,
                document_json = ?5, updated_at = ?6
            WHERE id = ?1 AND revision = ?7
            "#,
            params![
                draft.id.to_string(),
                &draft.spec.flow_id,
                i64::from(draft.revision),
                draft.status.as_str(),
                serde_json::to_string(draft)?,
                draft.updated_at.to_rfc3339(),
                i64::from(expected_revision),
            ],
        )?;
        if changed != 1 {
            let current: Option<i64> = conn
                .query_row(
                    "SELECT revision FROM flow_drafts WHERE id = ?1",
                    params![draft.id.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            return match current {
                Some(revision) => Err(FlowStoreError::RevisionConflict(
                    u32::try_from(revision).unwrap_or(u32::MAX),
                )
                .into()),
                None => Err(FlowStoreError::DraftNotFound(draft.id).into()),
            };
        }
        Ok(draft.clone())
    }

    fn bind_thread_flow_draft(&self, thread_id: Uuid, draft_id: Uuid) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let belongs_to_thread: bool = conn
            .query_row(
                "SELECT thread_id = ?2 FROM flow_drafts WHERE id = ?1",
                params![draft_id.to_string(), thread_id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(FlowStoreError::DraftNotFound(draft_id))?;
        if !belongs_to_thread {
            anyhow::bail!("Flow draft belongs to a different thread");
        }
        conn.execute(
            r#"
            INSERT INTO thread_flow_drafts (thread_id, draft_id, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(thread_id) DO UPDATE SET
                draft_id = excluded.draft_id,
                updated_at = excluded.updated_at
            "#,
            params![
                thread_id.to_string(),
                draft_id.to_string(),
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    fn get_thread_flow_draft(&self, thread_id: Uuid) -> anyhow::Result<Option<FlowDraftV1>> {
        let conn = self.read_connection();
        let document: Option<String> = conn
            .query_row(
                r#"
                SELECT flow_drafts.document_json
                FROM thread_flow_drafts
                JOIN flow_drafts ON flow_drafts.id = thread_flow_drafts.draft_id
                WHERE thread_flow_drafts.thread_id = ?1
                "#,
                params![thread_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        document
            .map(|document| serde_json::from_str(&document).map_err(Into::into))
            .transpose()
    }

    fn insert_flow_trial(&self, trial: &FlowTrialV1) -> anyhow::Result<FlowTrialV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO flow_trials (
                id, draft_id, draft_revision, status, document_json, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                trial.id.to_string(),
                trial.draft_id.to_string(),
                i64::from(trial.draft_revision),
                trial.status.as_str(),
                serde_json::to_string(trial)?,
                trial.created_at.to_rfc3339(),
            ],
        )?;
        Ok(trial.clone())
    }

    fn list_flow_trials(&self, draft_id: Uuid) -> anyhow::Result<Vec<FlowTrialV1>> {
        let conn = self.read_connection();
        let mut stmt = conn.prepare(
            "SELECT document_json FROM flow_trials WHERE draft_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(
            params![draft_id.to_string()],
            deserialize_json_column::<FlowTrialV1>,
        )?;
        collect_rows(rows)
    }

    fn publish_flow_draft(
        &self,
        draft_id: Uuid,
        published_by: &str,
    ) -> anyhow::Result<FlowDefinitionV1> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let mut draft =
            query_flow_draft(&tx, draft_id)?.ok_or(FlowStoreError::DraftNotFound(draft_id))?;
        if !draft
            .last_validation
            .as_ref()
            .is_some_and(|report| report.valid)
        {
            return Err(FlowStoreError::ValidationRequired.into());
        }
        let passed_trials: i64 = tx.query_row(
            r#"
            SELECT COUNT(*) FROM flow_trials
            WHERE draft_id = ?1 AND draft_revision = ?2 AND status = 'passed'
            "#,
            params![draft_id.to_string(), i64::from(draft.revision)],
            |row| row.get(0),
        )?;
        if passed_trials == 0 {
            return Err(FlowStoreError::PassedTrialRequired.into());
        }
        if matches!(
            draft.spec.risk_class,
            crate::enterprise::AgentRiskClassV1::High
                | crate::enterprise::AgentRiskClassV1::Critical
        ) && draft.spec.owner == published_by
        {
            return Err(FlowStoreError::IndependentApproverRequired.into());
        }
        let next_version: i64 = tx.query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM flow_definitions WHERE flow_id = ?1",
            params![&draft.spec.flow_id],
            |row| row.get(0),
        )?;
        let definition = definition_from_draft(
            &draft,
            u32::try_from(next_version).context("Flow version overflow")?,
            published_by,
        );
        tx.execute(
            r#"
            INSERT INTO flow_definitions (
                id, flow_id, version, name, owner, content_hash,
                document_json, published_at, published_by
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                definition.id.to_string(),
                &definition.flow_id,
                i64::from(definition.version),
                &definition.name,
                &definition.owner,
                &definition.content_hash,
                serde_json::to_string(&definition)?,
                definition.published_at.to_rfc3339(),
                &definition.published_by,
            ],
        )?;
        draft.status = FlowDraftStatusV1::Published;
        draft.updated_at = Utc::now();
        tx.execute(
            r#"
            UPDATE flow_drafts SET status = 'published', document_json = ?2, updated_at = ?3
            WHERE id = ?1
            "#,
            params![
                draft.id.to_string(),
                serde_json::to_string(&draft)?,
                draft.updated_at.to_rfc3339(),
            ],
        )?;
        tx.commit()?;
        Ok(definition)
    }

    fn search_flow_definitions(&self, query: &str) -> anyhow::Result<Vec<FlowDefinitionV1>> {
        let conn = self.read_connection();
        let pattern = format!("%{}%", query.trim());
        let mut stmt = conn.prepare(
            r#"
            SELECT document_json FROM flow_definitions
            WHERE flow_id LIKE ?1 OR name LIKE ?1 OR owner LIKE ?1
            ORDER BY published_at DESC
            LIMIT 100
            "#,
        )?;
        let rows = stmt.query_map(
            params![pattern],
            deserialize_json_column::<FlowDefinitionV1>,
        )?;
        collect_rows(rows)
    }

    fn get_flow_definition(
        &self,
        flow_id: &str,
        version: Option<u32>,
    ) -> anyhow::Result<Option<FlowDefinitionV1>> {
        let conn = self.read_connection();
        let document: Option<String> = match version {
            Some(version) => conn
                .query_row(
                    "SELECT document_json FROM flow_definitions WHERE flow_id = ?1 AND version = ?2",
                    params![flow_id, i64::from(version)],
                    |row| row.get(0),
                )
                .optional()?,
            None => conn
                .query_row(
                    "SELECT document_json FROM flow_definitions WHERE flow_id = ?1 ORDER BY version DESC LIMIT 1",
                    params![flow_id],
                    |row| row.get(0),
                )
                .optional()?,
        };
        document
            .map(|document| serde_json::from_str(&document).map_err(Into::into))
            .transpose()
    }

    fn insert_workflow_deployment(
        &self,
        deployment: &WorkflowDeploymentV1,
    ) -> anyhow::Result<WorkflowDeploymentV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let workflow = &deployment.snapshot.compiled_workflow;
        conn.execute(
            r#"
            INSERT INTO workflow_deployments (
                id, revision, name, environment, status, flow_id, flow_version,
                definition_id, snapshot_hash, document_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                deployment.id.to_string(),
                i64::from(deployment.revision),
                &deployment.name,
                &deployment.environment,
                deployment.status.as_str(),
                &workflow.flow_id,
                i64::from(workflow.flow_version),
                workflow.definition_id.to_string(),
                &deployment.snapshot.content_hash,
                serde_json::to_string(deployment)?,
                deployment.created_at.to_rfc3339(),
                deployment.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(deployment.clone())
    }

    fn get_workflow_deployment(
        &self,
        deployment_id: Uuid,
    ) -> anyhow::Result<Option<WorkflowDeploymentV1>> {
        let conn = self.read_connection();
        let document = conn
            .query_row(
                "SELECT document_json FROM workflow_deployments WHERE id = ?1",
                params![deployment_id.to_string()],
                deserialize_json_column::<WorkflowDeploymentV1>,
            )
            .optional()?;
        Ok(document)
    }

    fn list_workflow_deployments(
        &self,
        flow_id: Option<&str>,
        status: Option<WorkflowDeploymentStatusV1>,
    ) -> anyhow::Result<Vec<WorkflowDeploymentV1>> {
        let conn = self.read_connection();
        match (flow_id, status) {
            (Some(flow_id), Some(status)) => {
                let mut statement = conn.prepare(
                    "SELECT document_json FROM workflow_deployments WHERE flow_id = ?1 AND status = ?2 ORDER BY updated_at DESC",
                )?;
                let rows = statement.query_map(
                    params![flow_id, status.as_str()],
                    deserialize_json_column::<WorkflowDeploymentV1>,
                )?;
                collect_rows(rows)
            }
            (Some(flow_id), None) => {
                let mut statement = conn.prepare(
                    "SELECT document_json FROM workflow_deployments WHERE flow_id = ?1 ORDER BY updated_at DESC",
                )?;
                let rows = statement.query_map(
                    params![flow_id],
                    deserialize_json_column::<WorkflowDeploymentV1>,
                )?;
                collect_rows(rows)
            }
            (None, Some(status)) => {
                let mut statement = conn.prepare(
                    "SELECT document_json FROM workflow_deployments WHERE status = ?1 ORDER BY updated_at DESC",
                )?;
                let rows = statement.query_map(
                    params![status.as_str()],
                    deserialize_json_column::<WorkflowDeploymentV1>,
                )?;
                collect_rows(rows)
            }
            (None, None) => {
                let mut statement = conn.prepare(
                    "SELECT document_json FROM workflow_deployments ORDER BY updated_at DESC",
                )?;
                let rows =
                    statement.query_map([], deserialize_json_column::<WorkflowDeploymentV1>)?;
                collect_rows(rows)
            }
        }
    }

    fn update_workflow_deployment(
        &self,
        deployment: &WorkflowDeploymentV1,
        expected_revision: u32,
    ) -> anyhow::Result<WorkflowDeploymentV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let changed = conn.execute(
            r#"
            UPDATE workflow_deployments
            SET revision = ?2, status = ?3, document_json = ?4, updated_at = ?5
            WHERE id = ?1 AND revision = ?6
            "#,
            params![
                deployment.id.to_string(),
                i64::from(deployment.revision),
                deployment.status.as_str(),
                serde_json::to_string(deployment)?,
                deployment.updated_at.to_rfc3339(),
                i64::from(expected_revision),
            ],
        )?;
        if changed == 0 {
            let current = conn
                .query_row(
                    "SELECT revision FROM workflow_deployments WHERE id = ?1",
                    params![deployment.id.to_string()],
                    |row| row.get::<_, u32>(0),
                )
                .optional()?;
            return Err(match current {
                Some(revision) => WorkflowDeploymentStoreError::RevisionConflict(revision).into(),
                None => WorkflowDeploymentStoreError::NotFound(deployment.id).into(),
            });
        }
        Ok(deployment.clone())
    }

    fn insert_flow_run(&self, run: &FlowRunV1) -> anyhow::Result<FlowRunV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO flow_runs (
                id, thread_id, flow_id, flow_version, revision, status,
                document_json, created_at, updated_at, completed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                run.id.to_string(),
                run.thread_id.to_string(),
                &run.flow_id,
                i64::from(run.flow_version),
                i64::from(run.revision),
                run.status.as_str(),
                serde_json::to_string(run)?,
                run.created_at.to_rfc3339(),
                run.updated_at.to_rfc3339(),
                run.completed_at.map(|value| value.to_rfc3339()),
            ],
        )?;
        Ok(run.clone())
    }

    fn get_flow_run(&self, run_id: Uuid) -> anyhow::Result<Option<FlowRunV1>> {
        let conn = self.read_connection();
        let document: Option<String> = conn
            .query_row(
                "SELECT document_json FROM flow_runs WHERE id = ?1",
                params![run_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        document
            .map(|document| serde_json::from_str(&document).map_err(Into::into))
            .transpose()
    }

    fn list_flow_runs(&self, thread_id: Uuid) -> anyhow::Result<Vec<FlowRunV1>> {
        let conn = self.read_connection();
        let mut stmt = conn.prepare(
            "SELECT document_json FROM flow_runs WHERE thread_id = ?1 ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map(
            params![thread_id.to_string()],
            deserialize_json_column::<FlowRunV1>,
        )?;
        collect_rows(rows)
    }

    fn update_flow_run(
        &self,
        run: &FlowRunV1,
        expected_revision: u32,
    ) -> anyhow::Result<FlowRunV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        update_flow_run_conn(&conn, run, expected_revision)?;
        Ok(run.clone())
    }

    fn insert_human_task(&self, task: &HumanTaskV1) -> anyhow::Result<HumanTaskV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        insert_human_task_conn(&conn, task)?;
        touch_thread(&conn, task.thread_id)?;
        Ok(task.clone())
    }

    fn get_human_task(&self, task_id: Uuid) -> anyhow::Result<Option<HumanTaskV1>> {
        let conn = self.read_connection();
        let task = conn
            .query_row(
                "SELECT document_json FROM human_tasks WHERE id = ?1",
                params![task_id.to_string()],
                deserialize_json_column::<HumanTaskV1>,
            )
            .optional()?;
        Ok(task)
    }

    fn list_human_tasks(
        &self,
        thread_id: Option<Uuid>,
        status: Option<HumanTaskStatusV1>,
    ) -> anyhow::Result<Vec<HumanTaskV1>> {
        let conn = self.read_connection();
        match (thread_id, status) {
            (Some(thread_id), Some(status)) => {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT document_json FROM human_tasks
                    WHERE thread_id = ?1 AND status = ?2
                    ORDER BY updated_at DESC
                    "#,
                )?;
                let rows = stmt.query_map(
                    params![thread_id.to_string(), status.as_str()],
                    deserialize_json_column::<HumanTaskV1>,
                )?;
                collect_rows(rows)
            }
            (Some(thread_id), None) => {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT document_json FROM human_tasks
                    WHERE thread_id = ?1
                    ORDER BY updated_at DESC
                    "#,
                )?;
                let rows = stmt.query_map(
                    params![thread_id.to_string()],
                    deserialize_json_column::<HumanTaskV1>,
                )?;
                collect_rows(rows)
            }
            (None, Some(status)) => {
                let mut stmt = conn.prepare(
                    r#"
                    SELECT document_json FROM human_tasks
                    WHERE status = ?1
                    ORDER BY updated_at DESC
                    "#,
                )?;
                let rows = stmt.query_map(
                    params![status.as_str()],
                    deserialize_json_column::<HumanTaskV1>,
                )?;
                collect_rows(rows)
            }
            (None, None) => {
                let mut stmt =
                    conn.prepare("SELECT document_json FROM human_tasks ORDER BY updated_at DESC")?;
                let rows = stmt.query_map([], deserialize_json_column::<HumanTaskV1>)?;
                collect_rows(rows)
            }
        }
    }

    fn get_pending_human_task_for_flow_run(
        &self,
        flow_run_id: Uuid,
    ) -> anyhow::Result<Option<HumanTaskV1>> {
        let conn = self.read_connection();
        let task = conn
            .query_row(
                r#"
                SELECT document_json FROM human_tasks
                WHERE source_kind = 'flow_run' AND source_id = ?1 AND status = 'pending'
                ORDER BY created_at ASC
                LIMIT 1
                "#,
                params![flow_run_id.to_string()],
                deserialize_json_column::<HumanTaskV1>,
            )
            .optional()?;
        Ok(task)
    }

    fn update_human_task(
        &self,
        task: &HumanTaskV1,
        expected_revision: u32,
    ) -> anyhow::Result<HumanTaskV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        update_human_task_conn(&conn, task, expected_revision)?;
        touch_thread(&conn, task.thread_id)?;
        Ok(task.clone())
    }

    fn update_flow_run_and_human_task(
        &self,
        run: &FlowRunV1,
        expected_run_revision: u32,
        task: &HumanTaskV1,
        expected_task_revision: Option<u32>,
    ) -> anyhow::Result<(FlowRunV1, HumanTaskV1)> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        update_flow_run_conn(&transaction, run, expected_run_revision)?;
        match expected_task_revision {
            Some(expected_revision) => {
                update_human_task_conn(&transaction, task, expected_revision)?;
            }
            None => insert_human_task_conn(&transaction, task)?,
        }
        touch_thread(&transaction, run.thread_id)?;
        transaction.commit()?;
        Ok((run.clone(), task.clone()))
    }

    fn create_project(
        &self,
        name: String,
        workspace_root: Option<PathBuf>,
        pinned: bool,
        sort_order: i64,
    ) -> anyhow::Result<Project> {
        let name = validated_project_name(name)?;
        let (workspace_root_value, workspace_key) = project_workspace_values(&workspace_root)?;
        let mut project = Project::new(name, workspace_root);
        project.pinned = pinned;
        project.sort_order = sort_order;

        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        ensure_workspace_available(&conn, workspace_key.as_deref(), None)?;
        insert_project(
            &conn,
            &project,
            workspace_root_value.as_deref(),
            workspace_key.as_deref(),
        )?;
        Ok(project)
    }

    fn get_project(&self, id: Uuid) -> anyhow::Result<Option<Project>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        query_project(&conn, id)
    }

    fn find_project_by_workspace(&self, workspace_root: &Path) -> anyhow::Result<Option<Project>> {
        let key = validated_workspace_key(workspace_root)?;
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        query_project_by_workspace_key(&conn, &key)
    }

    fn find_or_create_project(
        &self,
        name: String,
        workspace_root: PathBuf,
    ) -> anyhow::Result<Project> {
        let name = validated_project_name(name)?;
        let workspace_key = validated_workspace_key(&workspace_root)?;
        let workspace_root_value = workspace_root.to_string_lossy().into_owned();
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        if let Some(project) = query_project_by_workspace_key(&conn, &workspace_key)? {
            return Ok(project);
        }

        let sort_order = conn.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM projects",
            [],
            |row| row.get(0),
        )?;
        let mut project = Project::new(name, Some(workspace_root));
        project.sort_order = sort_order;
        insert_project(
            &conn,
            &project,
            Some(&workspace_root_value),
            Some(&workspace_key),
        )?;
        Ok(project)
    }

    fn list_projects(&self) -> anyhow::Result<Vec<Project>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT id, name, workspace_root, pinned, sort_order, created_at, updated_at
            FROM projects
            ORDER BY pinned DESC, sort_order ASC, created_at ASC
            "#,
        )?;
        let rows = stmt.query_map([], map_project)?;
        collect_rows(rows)
    }

    fn update_project(
        &self,
        id: Uuid,
        name: Option<String>,
        workspace_root: Option<Option<PathBuf>>,
        pinned: Option<bool>,
        sort_order: Option<i64>,
    ) -> anyhow::Result<Option<Project>> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let Some(mut project) = query_project(&tx, id)? else {
            return Ok(None);
        };

        if let Some(name) = name {
            project.name = validated_project_name(name)?;
        }
        if let Some(workspace_root) = workspace_root {
            let (_, workspace_key) = project_workspace_values(&workspace_root)?;
            ensure_workspace_available(&tx, workspace_key.as_deref(), Some(id))?;
            if workspace_root.is_none() {
                let thread_count: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM threads WHERE project_id = ?1",
                    params![id.to_string()],
                    |row| row.get(0),
                )?;
                if thread_count > 0 {
                    return Err(StoreError::ProjectWorkspaceInUse(id).into());
                }
            }
            project.workspace_root = workspace_root;
        }
        if let Some(pinned) = pinned {
            project.pinned = pinned;
        }
        if let Some(sort_order) = sort_order {
            project.sort_order = sort_order;
        }
        project.updated_at = Utc::now();
        let (workspace_root_value, workspace_key) =
            project_workspace_values(&project.workspace_root)?;
        tx.execute(
            r#"
            UPDATE projects
            SET name = ?1, workspace_root = ?2, workspace_key = ?3,
                pinned = ?4, sort_order = ?5, updated_at = ?6
            WHERE id = ?7
            "#,
            params![
                &project.name,
                workspace_root_value,
                workspace_key,
                project.pinned as i64,
                project.sort_order,
                project.updated_at.to_rfc3339(),
                id.to_string(),
            ],
        )?;
        if let Some(workspace_root) = project.workspace_root.as_ref() {
            tx.execute(
                r#"
                UPDATE threads
                SET workspace_root = ?1, updated_at = ?2
                WHERE project_id = ?3 AND workspace_root != ?1
                "#,
                params![
                    workspace_root.to_string_lossy(),
                    project.updated_at.to_rfc3339(),
                    id.to_string(),
                ],
            )?;
        }
        tx.commit()?;
        Ok(Some(project))
    }

    fn delete_project(&self, id: Uuid) -> anyhow::Result<bool> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        if query_project(&tx, id)?.is_none() {
            return Ok(false);
        }
        let archived_at = Utc::now().to_rfc3339();
        tx.execute(
            r#"
            UPDATE threads
            SET project_id = NULL,
                archived_at = COALESCE(archived_at, ?1),
                updated_at = ?1
            WHERE project_id = ?2
            "#,
            params![archived_at, id.to_string()],
        )?;
        let deleted = tx.execute(
            "DELETE FROM projects WHERE id = ?1",
            params![id.to_string()],
        )?;
        tx.commit()?;
        Ok(deleted > 0)
    }

    fn create_thread(
        &self,
        title: Option<String>,
        workspace_root: PathBuf,
    ) -> anyhow::Result<Thread> {
        self.create_thread_with_mode(title, workspace_root, ExperienceMode::Code)
    }

    fn create_thread_with_mode(
        &self,
        title: Option<String>,
        workspace_root: PathBuf,
        experience_mode: ExperienceMode,
    ) -> anyhow::Result<Thread> {
        let thread = Thread::new_with_mode(
            title.unwrap_or_else(|| "Untitled thread".to_string()),
            workspace_root,
            experience_mode,
        );
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        insert_thread(&conn, &thread)?;
        Ok(thread)
    }

    fn create_thread_in_project(
        &self,
        title: Option<String>,
        project_id: Uuid,
    ) -> anyhow::Result<Thread> {
        self.create_thread_in_project_with_mode(title, project_id, ExperienceMode::Code)
    }

    fn create_thread_in_project_with_mode(
        &self,
        title: Option<String>,
        project_id: Uuid,
        experience_mode: ExperienceMode,
    ) -> anyhow::Result<Thread> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let project =
            query_project(&conn, project_id)?.ok_or(StoreError::ProjectNotFound(project_id))?;
        let workspace_root = project
            .workspace_root
            .ok_or(StoreError::ProjectHasNoWorkspace(project_id))?;
        let thread = Thread::new_in_project_with_mode(
            title.unwrap_or_else(|| "Untitled thread".to_string()),
            workspace_root,
            project_id,
            experience_mode,
        );
        insert_thread(&conn, &thread)?;
        Ok(thread)
    }

    fn get_thread(&self, id: Uuid) -> anyhow::Result<Option<Thread>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let thread = conn
            .query_row(
                r#"
                SELECT id, title, workspace_root, project_id, archived_at, experience_mode, model_selection, created_at, updated_at
                FROM threads
                WHERE id = ?1
                "#,
                params![id.to_string()],
                map_thread,
            )
            .optional()?;
        Ok(thread)
    }

    fn list_threads(&self) -> anyhow::Result<Vec<Thread>> {
        self.list_threads_including_archived(false)
    }

    fn list_threads_including_archived(
        &self,
        include_archived: bool,
    ) -> anyhow::Result<Vec<Thread>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let sql = if include_archived {
            r#"
            SELECT id, title, workspace_root, project_id, archived_at, experience_mode, model_selection, created_at, updated_at
            FROM threads
            ORDER BY updated_at DESC
            "#
        } else {
            r#"
            SELECT id, title, workspace_root, project_id, archived_at, experience_mode, model_selection, created_at, updated_at
            FROM threads
            WHERE archived_at IS NULL
            ORDER BY updated_at DESC
            "#
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], map_thread)?;
        collect_rows(rows)
    }

    fn list_threads_for_mode(
        &self,
        include_archived: bool,
        experience_mode: ExperienceMode,
    ) -> anyhow::Result<Vec<Thread>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let sql = if include_archived {
            r#"
            SELECT id, title, workspace_root, project_id, archived_at, experience_mode, model_selection, created_at, updated_at
            FROM threads
            WHERE experience_mode = ?1
            ORDER BY updated_at DESC
            "#
        } else {
            r#"
            SELECT id, title, workspace_root, project_id, archived_at, experience_mode, model_selection, created_at, updated_at
            FROM threads
            WHERE archived_at IS NULL AND experience_mode = ?1
            ORDER BY updated_at DESC
            "#
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params![experience_mode.as_str()], map_thread)?;
        collect_rows(rows)
    }

    fn update_thread(
        &self,
        id: Uuid,
        title: Option<String>,
        project_id: Option<Option<Uuid>>,
        archived: Option<bool>,
    ) -> anyhow::Result<Option<Thread>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let Some(mut thread) = query_thread(&conn, id)? else {
            return Ok(None);
        };
        if let Some(title) = title {
            let title = title.trim();
            if title.is_empty() {
                return Err(StoreError::EmptyThreadTitle.into());
            }
            thread.title = title.to_string();
        }
        if let Some(project_id) = project_id {
            match project_id {
                Some(project_id) => {
                    let project = query_project(&conn, project_id)?
                        .ok_or(StoreError::ProjectNotFound(project_id))?;
                    let workspace_root = project
                        .workspace_root
                        .ok_or(StoreError::ProjectHasNoWorkspace(project_id))?;
                    thread.project_id = Some(project_id);
                    thread.workspace_root = workspace_root;
                }
                None => thread.project_id = None,
            }
        }
        if let Some(archived) = archived {
            thread.archived_at = archived.then(Utc::now);
        }
        thread.updated_at = Utc::now();
        conn.execute(
            r#"
            UPDATE threads
            SET title = ?1, workspace_root = ?2, project_id = ?3,
                archived_at = ?4, updated_at = ?5
            WHERE id = ?6
            "#,
            params![
                &thread.title,
                thread.workspace_root.to_string_lossy(),
                thread.project_id.map(|value| value.to_string()),
                thread.archived_at.map(|value| value.to_rfc3339()),
                thread.updated_at.to_rfc3339(),
                id.to_string(),
            ],
        )?;
        Ok(Some(thread))
    }

    fn set_thread_model_selection(
        &self,
        id: Uuid,
        selection: Option<ThreadModelSelection>,
    ) -> anyhow::Result<Option<Thread>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let Some(mut thread) = query_thread(&conn, id)? else {
            return Ok(None);
        };
        thread.model_selection = selection;
        thread.updated_at = Utc::now();
        conn.execute(
            r#"
            UPDATE threads
            SET model_selection = ?1, updated_at = ?2
            WHERE id = ?3
            "#,
            params![
                encode_model_selection(thread.model_selection.as_ref())?,
                thread.updated_at.to_rfc3339(),
                id.to_string(),
            ],
        )?;
        Ok(Some(thread))
    }

    fn delete_thread(&self, id: Uuid) -> anyhow::Result<bool> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let deleted = conn.execute("DELETE FROM threads WHERE id = ?1", params![id.to_string()])?;
        Ok(deleted > 0)
    }

    fn create_goal(
        &self,
        thread_id: Uuid,
        objective: String,
        token_budget: Option<u64>,
    ) -> anyhow::Result<GoalSnapshot> {
        let objective = objective.trim().to_string();
        anyhow::ensure!(!objective.is_empty(), "goal objective cannot be empty");
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        anyhow::ensure!(
            query_thread(&conn, thread_id)?.is_some(),
            "thread not found: {thread_id}"
        );
        let goal = GoalRecord::new(thread_id, objective, token_budget);
        conn.execute(
            r#"
            INSERT INTO goals (
                id, thread_id, objective, token_budget, tokens_used,
                time_used_seconds, version, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                goal.id.to_string(),
                goal.thread_id.to_string(),
                &goal.objective,
                goal.token_budget.map(|value| value as i64),
                goal.tokens_used as i64,
                goal.time_used_seconds as i64,
                goal.version as i64,
                goal.created_at.to_rfc3339(),
                goal.updated_at.to_rfc3339(),
            ],
        )?;
        let form = WorkForm::empty_goal(thread_id, goal.id, goal.objective.clone());
        upsert_work_form_conn(&conn, &form)?;
        load_goal_snapshot(&conn, goal.id)?.context("created goal disappeared")
    }

    fn get_goal(&self, id: Uuid) -> anyhow::Result<Option<GoalSnapshot>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        load_goal_snapshot(&conn, id)
    }

    fn get_thread_goal(&self, thread_id: Uuid) -> anyhow::Result<Option<GoalSnapshot>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let goal_ids = {
            let mut stmt = conn.prepare(
                "SELECT id FROM goals WHERE thread_id = ?1 ORDER BY updated_at DESC, rowid DESC",
            )?;
            let ids = collect_rows(stmt.query_map(params![thread_id.to_string()], |row| {
                row.get::<_, String>(0)
            })?)?;
            ids
        };
        let mut latest_terminal = None;
        for id in goal_ids {
            let id = Uuid::parse_str(&id)?;
            let Some(snapshot) = load_goal_snapshot(&conn, id)? else {
                continue;
            };
            if !snapshot.status().is_terminal() {
                return Ok(Some(snapshot));
            }
            latest_terminal.get_or_insert(snapshot);
        }
        Ok(latest_terminal)
    }

    fn update_goal_status(
        &self,
        thread_id: Uuid,
        goal_id: Uuid,
        status: GoalStatus,
    ) -> anyhow::Result<Option<GoalSnapshot>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let current = match load_goal_snapshot(&conn, goal_id)? {
            Some(goal) if goal.goal.thread_id == thread_id => goal,
            Some(_) => anyhow::bail!("goal {goal_id} does not belong to thread {thread_id}"),
            None => return Ok(None),
        };
        anyhow::ensure!(
            valid_goal_transition(current.status(), status),
            "invalid goal transition: {} -> {}",
            current.status().as_str(),
            status.as_str()
        );
        let now = Utc::now();
        conn.execute(
            r#"
            UPDATE goals
            SET updated_at = ?3, version = version + 1
            WHERE id = ?1 AND thread_id = ?2
            "#,
            params![goal_id.to_string(), thread_id.to_string(), now.to_rfc3339(),],
        )?;
        if let Some(mut form) = query_work_form_for_scope(&conn, WorkScope::Goal(goal_id))? {
            form.set_status(status);
            upsert_work_form_conn(&conn, &form)?;
        }
        load_goal_snapshot(&conn, goal_id)
    }

    fn update_goal_definition(
        &self,
        thread_id: Uuid,
        goal_id: Uuid,
        objective: Option<String>,
        constraints: Option<Vec<String>>,
        acceptance: Option<Vec<String>>,
    ) -> anyhow::Result<Option<GoalSnapshot>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let Some(snapshot) = load_goal_snapshot(&conn, goal_id)? else {
            return Ok(None);
        };
        anyhow::ensure!(
            snapshot.goal.thread_id == thread_id,
            "goal {goal_id} does not belong to thread {thread_id}"
        );
        let mut form = snapshot.work_form;
        if let Some(objective) = objective {
            let objective = objective.trim().to_string();
            anyhow::ensure!(!objective.is_empty(), "goal objective cannot be empty");
            anyhow::ensure!(
                objective.chars().count() <= 300,
                "goal objective exceeds 300 characters"
            );
            form.objective = objective;
        }
        if let Some(constraints) = constraints {
            form.constraints = validate_goal_definition_list("constraints", constraints)?;
        }
        if let Some(acceptance) = acceptance {
            form.acceptance = validate_goal_definition_list("acceptance", acceptance)?;
        }
        form.revision = form
            .revision
            .checked_add(1)
            .context("WorkForm revision overflow")?;
        form.change_reason = Some("explicit goal definition edit".to_string());
        form.updated_at = Utc::now();
        form.validate()?;
        upsert_work_form_conn(&conn, &form)?;
        conn.execute(
            "UPDATE goals SET updated_at = ?2, version = version + 1 WHERE id = ?1",
            params![goal_id.to_string(), form.updated_at.to_rfc3339()],
        )?;
        load_goal_snapshot(&conn, goal_id)
    }

    fn upsert_work_form(&self, form: &WorkForm) -> anyhow::Result<WorkForm> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        anyhow::ensure!(
            query_thread(&conn, form.thread_id)?.is_some(),
            "thread not found: {}",
            form.thread_id
        );
        upsert_work_form_conn(&conn, form)?;
        Ok(form.clone())
    }

    fn get_work_form(&self, form_id: Uuid) -> anyhow::Result<Option<WorkForm>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        query_work_form(&conn, form_id)
    }

    fn get_work_form_for_scope(&self, scope: WorkScope) -> anyhow::Result<Option<WorkForm>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        query_work_form_for_scope(&conn, scope)
    }

    fn add_goal_usage(
        &self,
        goal_id: Uuid,
        tokens: u64,
        elapsed_seconds: u64,
    ) -> anyhow::Result<Option<GoalSnapshot>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let changed = conn.execute(
            r#"
            UPDATE goals
            SET tokens_used = tokens_used + ?2,
                time_used_seconds = time_used_seconds + ?3,
                updated_at = ?4,
                version = version + 1
            WHERE id = ?1
            "#,
            params![
                goal_id.to_string(),
                i64::try_from(tokens).context("goal token usage exceeds SQLite range")?,
                i64::try_from(elapsed_seconds).context("goal time usage exceeds SQLite range")?,
                Utc::now().to_rfc3339(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        load_goal_snapshot(&conn, goal_id)
    }

    fn append_message(&self, message: Message) -> anyhow::Result<Message> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let parts_json = serde_json::to_string(&message.parts)?;
        conn.execute(
            r#"
            INSERT INTO messages (id, thread_id, role, parts_json, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                message.id.to_string(),
                message.thread_id.to_string(),
                message.role.as_str(),
                parts_json,
                message.created_at.to_rfc3339(),
            ],
        )?;
        touch_thread(&conn, message.thread_id)?;
        Ok(message)
    }

    fn list_messages(&self, thread_id: Uuid) -> anyhow::Result<Vec<Message>> {
        let conn = self.read_connection();
        let mut stmt = conn.prepare(
            r#"
            SELECT id, thread_id, role, parts_json, created_at
            FROM messages
            WHERE thread_id = ?1
            ORDER BY created_at ASC, rowid ASC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], map_message)?;
        collect_rows(rows)
    }

    fn enqueue_turn_message(&self, thread_id: Uuid, message_id: Uuid) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO turn_queue (message_id, thread_id, queued_at)
            VALUES (?1, ?2, ?3)
            "#,
            params![
                message_id.to_string(),
                thread_id.to_string(),
                Utc::now().to_rfc3339(),
            ],
        )?;
        touch_thread(&conn, thread_id)?;
        Ok(())
    }

    fn list_queued_turn_messages(&self, thread_id: Uuid) -> anyhow::Result<Vec<Uuid>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT message_id
            FROM turn_queue
            WHERE thread_id = ?1
            ORDER BY queued_at ASC, rowid ASC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], |row| {
            let raw: String = row.get(0)?;
            Uuid::parse_str(&raw).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
            })
        })?;
        collect_rows(rows)
    }

    fn remove_queued_turn_message(
        &self,
        thread_id: Uuid,
        message_id: Uuid,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let removed = conn.execute(
            "DELETE FROM turn_queue WHERE thread_id = ?1 AND message_id = ?2",
            params![thread_id.to_string(), message_id.to_string()],
        )?;
        Ok(removed > 0)
    }

    fn insert_turn(&self, turn: TurnRecord) -> anyhow::Result<TurnRecord> {
        anyhow::ensure!(
            turn.status == TurnStatus::Running,
            "new turns must start in running status"
        );
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO turns (
                turn_id, invocation_id, thread_id, user_message_id, status,
                started_at, updated_at, completed_at, error
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                turn.turn_id.to_string(),
                turn.invocation_id as i64,
                turn.thread_id.to_string(),
                turn.user_message_id.to_string(),
                turn.status.as_str(),
                turn.started_at.to_rfc3339(),
                turn.updated_at.to_rfc3339(),
                turn.completed_at.map(|value| value.to_rfc3339()),
                turn.error.as_deref(),
            ],
        )?;
        touch_thread(&conn, turn.thread_id)?;
        Ok(turn)
    }

    fn get_turn(&self, turn_id: Uuid) -> anyhow::Result<Option<TurnRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            r#"
            SELECT turn_id, invocation_id, thread_id, user_message_id, status,
                   started_at, updated_at, completed_at, error
            FROM turns
            WHERE turn_id = ?1
            "#,
            params![turn_id.to_string()],
            map_turn,
        )
        .optional()
        .map_err(Into::into)
    }

    fn get_active_turn(&self, thread_id: Uuid) -> anyhow::Result<Option<TurnRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            r#"
            SELECT turn_id, invocation_id, thread_id, user_message_id, status,
                   started_at, updated_at, completed_at, error
            FROM turns
            WHERE thread_id = ?1 AND status IN ('running', 'cancelling')
            ORDER BY started_at DESC, rowid DESC
            LIMIT 1
            "#,
            params![thread_id.to_string()],
            map_turn,
        )
        .optional()
        .map_err(Into::into)
    }

    fn get_latest_turn(&self, thread_id: Uuid) -> anyhow::Result<Option<TurnRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            r#"
            SELECT turn_id, invocation_id, thread_id, user_message_id, status,
                   started_at, updated_at, completed_at, error
            FROM turns
            WHERE thread_id = ?1
            ORDER BY started_at DESC, rowid DESC
            LIMIT 1
            "#,
            params![thread_id.to_string()],
            map_turn,
        )
        .optional()
        .map_err(Into::into)
    }

    fn update_turn_status(
        &self,
        turn_id: Uuid,
        status: TurnStatus,
        error: Option<String>,
    ) -> anyhow::Result<Option<TurnRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let now = Utc::now();
        let completed_at = status.is_terminal().then(|| now.to_rfc3339());
        let changed = conn.execute(
            r#"
            UPDATE turns
            SET status = ?2, updated_at = ?3, completed_at = ?4, error = ?5
            WHERE turn_id = ?1
            "#,
            params![
                turn_id.to_string(),
                status.as_str(),
                now.to_rfc3339(),
                completed_at,
                error,
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        conn.query_row(
            r#"
            SELECT turn_id, invocation_id, thread_id, user_message_id, status,
                   started_at, updated_at, completed_at, error
            FROM turns
            WHERE turn_id = ?1
            "#,
            params![turn_id.to_string()],
            map_turn,
        )
        .optional()
        .map_err(Into::into)
    }

    fn resume_turn_invocation(&self, turn_id: Uuid) -> anyhow::Result<Option<TurnRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let now = Utc::now();
        let changed = conn.execute(
            r#"
            UPDATE turns
            SET invocation_id = invocation_id + 1, status = 'running',
                updated_at = ?2, completed_at = NULL, error = NULL
            WHERE turn_id = ?1 AND status IN (
                'waiting_approval', 'waiting_user_input', 'waiting_user_action'
            )
            "#,
            params![turn_id.to_string(), now.to_rfc3339()],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        conn.query_row(
            r#"
            SELECT turn_id, invocation_id, thread_id, user_message_id, status,
                   started_at, updated_at, completed_at, error
            FROM turns
            WHERE turn_id = ?1
            "#,
            params![turn_id.to_string()],
            map_turn,
        )
        .optional()
        .map_err(Into::into)
    }

    fn interrupt_active_turns(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let now = Utc::now().to_rfc3339();
        let changed = conn.execute(
            r#"
            UPDATE turns
            SET status = 'interrupted', updated_at = ?1, completed_at = ?1,
                error = 'server restarted before turn completed'
            WHERE status IN ('running', 'cancelling')
            "#,
            params![now],
        )?;
        Ok(changed)
    }

    fn upsert_turn_change_set(&self, change_set: &TurnChangeSet) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO turn_change_sets (
                turn_id, thread_id, workspace_root, repo_root, workspace_prefix,
                before_tree, after_tree, status, files_json, additions, deletions,
                error, created_at, finalized_at, reverted_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ON CONFLICT(turn_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                workspace_root = excluded.workspace_root,
                repo_root = excluded.repo_root,
                workspace_prefix = excluded.workspace_prefix,
                before_tree = excluded.before_tree,
                after_tree = excluded.after_tree,
                status = excluded.status,
                files_json = excluded.files_json,
                additions = excluded.additions,
                deletions = excluded.deletions,
                error = excluded.error,
                finalized_at = excluded.finalized_at,
                reverted_at = excluded.reverted_at
            "#,
            params![
                change_set.turn_id.to_string(),
                change_set.thread_id.to_string(),
                change_set.workspace_root.to_string_lossy(),
                change_set
                    .repo_root
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                change_set
                    .workspace_prefix
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                change_set.before_tree.as_deref(),
                change_set.after_tree.as_deref(),
                change_set.status.as_str(),
                serde_json::to_string(&change_set.files)?,
                i64::try_from(change_set.additions)
                    .context("turn additions exceed SQLite range")?,
                i64::try_from(change_set.deletions)
                    .context("turn deletions exceed SQLite range")?,
                change_set.error.as_deref(),
                change_set.created_at.to_rfc3339(),
                change_set.finalized_at.map(|value| value.to_rfc3339()),
                change_set.reverted_at.map(|value| value.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    fn get_turn_change_set(&self, turn_id: Uuid) -> anyhow::Result<Option<TurnChangeSet>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            r#"
            SELECT turn_id, thread_id, workspace_root, repo_root, workspace_prefix,
                   before_tree, after_tree, status, files_json, additions, deletions,
                   error, created_at, finalized_at, reverted_at
            FROM turn_change_sets
            WHERE turn_id = ?1
            "#,
            params![turn_id.to_string()],
            map_turn_change_set,
        )
        .optional()
        .map_err(Into::into)
    }

    fn list_turn_change_sets(&self, thread_id: Uuid) -> anyhow::Result<Vec<TurnChangeSet>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT turn_id, thread_id, workspace_root, repo_root, workspace_prefix,
                   before_tree, after_tree, status, files_json, additions, deletions,
                   error, created_at, finalized_at, reverted_at
            FROM turn_change_sets
            WHERE thread_id = ?1
            ORDER BY created_at ASC, rowid ASC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], map_turn_change_set)?;
        collect_rows(rows)
    }

    fn mark_turn_change_set_reverted(
        &self,
        turn_id: Uuid,
        reverted_at: DateTime<Utc>,
    ) -> anyhow::Result<Option<TurnChangeSet>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let changed = conn.execute(
            "UPDATE turn_change_sets SET reverted_at = ?2 WHERE turn_id = ?1",
            params![turn_id.to_string(), reverted_at.to_rfc3339()],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        conn.query_row(
            r#"
            SELECT turn_id, thread_id, workspace_root, repo_root, workspace_prefix,
                   before_tree, after_tree, status, files_json, additions, deletions,
                   error, created_at, finalized_at, reverted_at
            FROM turn_change_sets
            WHERE turn_id = ?1
            "#,
            params![turn_id.to_string()],
            map_turn_change_set,
        )
        .optional()
        .map_err(Into::into)
    }

    fn append_event(&self, event: AgentEvent) -> anyhow::Result<AgentEvent> {
        self.append_events(vec![event])?
            .into_iter()
            .next()
            .context("single-event append returned no event")
    }

    fn append_events(&self, mut events: Vec<AgentEvent>) -> anyhow::Result<Vec<AgentEvent>> {
        let Some(thread_id) = events.first().map(|event| event.thread_id) else {
            return Ok(events);
        };
        anyhow::ensure!(
            events.iter().all(|event| event.thread_id == thread_id),
            "an event batch must belong to one thread"
        );

        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let first_seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE thread_id = ?1",
            params![thread_id.to_string()],
            |row| row.get(0),
        )?;
        let mut completed_stream_turn_ids = HashSet::new();
        for (offset, event) in events.iter_mut().enumerate() {
            event.seq = first_seq + i64::try_from(offset)?;
            let payload_json = serde_json::to_string(&event.payload)?;
            let conversation_payload_json =
                conversation_payload_json(&event.payload, &payload_json)?;
            tx.execute(
                r#"
                INSERT INTO events (id, thread_id, turn_id, seq, kind, payload_json, created_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    event.id.to_string(),
                    event.thread_id.to_string(),
                    event.turn_id.as_ref().map(|id| id.to_string()),
                    event.seq,
                    event.kind(),
                    payload_json,
                    event.created_at.to_rfc3339(),
                ],
            )?;
            if let Some(conversation_payload_json) = conversation_payload_json {
                tx.execute(
                    r#"
                    INSERT INTO conversation_events (
                        id, thread_id, turn_id, seq, payload_json, created_at
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    "#,
                    params![
                        event.id.to_string(),
                        event.thread_id.to_string(),
                        event.turn_id.as_ref().map(|id| id.to_string()),
                        event.seq,
                        conversation_payload_json,
                        event.created_at.to_rfc3339(),
                    ],
                )?;
            }
            if matches!(&event.payload, AgentEventPayload::AssistantMessage { .. }) {
                if let Some(turn_id) = event.turn_id {
                    completed_stream_turn_ids.insert(turn_id);
                }
            }
        }
        // The assistant message is the durable snapshot of a completed model
        // stream. Historical deltas remain in the diagnostic event log, while
        // the conversation projection stays small and quick to restore.
        for turn_id in completed_stream_turn_ids {
            tx.execute(
                r#"
                DELETE FROM conversation_events
                WHERE thread_id = ?1
                  AND turn_id = ?2
                  AND id IN (SELECT id FROM events WHERE kind = 'model_delta')
                "#,
                params![thread_id.to_string(), turn_id.to_string()],
            )?;
        }
        touch_thread(&tx, thread_id)?;
        tx.commit()?;
        Ok(events)
    }

    fn list_events(
        &self,
        thread_id: Uuid,
        after_seq: Option<i64>,
    ) -> anyhow::Result<Vec<AgentEvent>> {
        let conn = self.read_connection();
        let mut stmt = conn.prepare(
            r#"
            SELECT id, thread_id, turn_id, seq, payload_json, created_at
            FROM events
            WHERE thread_id = ?1 AND seq > ?2
            ORDER BY seq ASC
            "#,
        )?;
        let rows = stmt.query_map(
            params![thread_id.to_string(), after_seq.unwrap_or(0)],
            map_event,
        )?;
        collect_rows(rows)
    }

    fn prepare_effect(&self, intent: &EffectIntent) -> anyhow::Result<EffectJournalRecord> {
        validate_effect_intent(intent)?;
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        if let Some(existing) = query_effect_by_idempotency_key(
            &conn,
            intent.thread_id,
            intent.turn_id,
            &intent.agent_path,
            &intent.idempotency_key,
        )? {
            if existing.kind != intent.kind
                || existing.operation != intent.operation
                || existing.input_hash != intent.input_hash
            {
                return Err(EffectJournalError::IdempotencyConflict {
                    key: intent.idempotency_key.clone(),
                }
                .into());
            }
            return Ok(existing);
        }

        let now = Utc::now();
        let effect_id = Uuid::new_v4();
        conn.execute(
            r#"
            INSERT INTO effect_journal (
                effect_id, thread_id, turn_id, agent_path, idempotency_key,
                kind, operation, input_hash, input_json, result_json, status,
                side_effect_class, idempotent, attempt, error, created_at,
                started_at, completed_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10,
                ?11, ?12, 0, NULL, ?13, NULL, NULL, ?13
            )
            "#,
            params![
                effect_id.to_string(),
                intent.thread_id.to_string(),
                intent.turn_id.to_string(),
                &intent.agent_path,
                &intent.idempotency_key,
                intent.kind.as_str(),
                &intent.operation,
                &intent.input_hash,
                serde_json::to_string(&intent.input)?,
                EffectStatus::Prepared.as_str(),
                intent.side_effect_class.as_str(),
                intent.idempotent,
                now.to_rfc3339(),
            ],
        )?;
        query_effect(&conn, effect_id)?.context("newly prepared effect was not persisted")
    }

    fn get_effect(&self, effect_id: Uuid) -> anyhow::Result<Option<EffectJournalRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        query_effect(&conn, effect_id)
    }

    fn get_effect_by_idempotency_key(
        &self,
        thread_id: Uuid,
        turn_id: Uuid,
        agent_path: &str,
        idempotency_key: &str,
    ) -> anyhow::Result<Option<EffectJournalRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        query_effect_by_idempotency_key(&conn, thread_id, turn_id, agent_path, idempotency_key)
    }

    fn list_turn_effects(&self, turn_id: Uuid) -> anyhow::Result<Vec<EffectJournalRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "{} WHERE turn_id = ?1 ORDER BY created_at ASC, effect_id ASC",
            effect_select_sql()
        ))?;
        let rows = stmt.query_map(params![turn_id.to_string()], map_effect)?;
        collect_rows(rows)
    }

    fn start_effect(&self, effect_id: Uuid) -> anyhow::Result<EffectJournalRecord> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let current = query_effect(&conn, effect_id)?.context("effect not found")?;
        if current.status == EffectStatus::Running {
            return Ok(current);
        }
        if !valid_effect_transition(current.status, EffectStatus::Running) {
            return Err(EffectJournalError::InvalidTransition {
                effect_id,
                from: current.status,
                to: EffectStatus::Running,
            }
            .into());
        }
        let now = Utc::now().to_rfc3339();
        conn.execute(
            r#"
            UPDATE effect_journal
            SET status = ?2, attempt = attempt + 1, started_at = ?3,
                completed_at = NULL, result_json = NULL, error = NULL, updated_at = ?3
            WHERE effect_id = ?1
            "#,
            params![effect_id.to_string(), EffectStatus::Running.as_str(), now],
        )?;
        query_effect(&conn, effect_id)?.context("started effect disappeared")
    }

    fn finish_effect(
        &self,
        effect_id: Uuid,
        status: EffectStatus,
        result: Option<Value>,
        error: Option<String>,
    ) -> anyhow::Result<EffectJournalRecord> {
        if !matches!(
            status,
            EffectStatus::Succeeded | EffectStatus::Failed | EffectStatus::Indeterminate
        ) {
            anyhow::bail!("finish_effect requires a terminal or indeterminate status");
        }
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let current = query_effect(&conn, effect_id)?.context("effect not found")?;
        if current.status == status {
            return Ok(current);
        }
        if !valid_effect_transition(current.status, status) {
            return Err(EffectJournalError::InvalidTransition {
                effect_id,
                from: current.status,
                to: status,
            }
            .into());
        }
        let now = Utc::now().to_rfc3339();
        let completed_at = status.is_terminal().then_some(now.clone());
        conn.execute(
            r#"
            UPDATE effect_journal
            SET status = ?2, result_json = ?3, error = ?4,
                completed_at = ?5, updated_at = ?6
            WHERE effect_id = ?1
            "#,
            params![
                effect_id.to_string(),
                status.as_str(),
                result.as_ref().map(serde_json::to_string).transpose()?,
                error,
                completed_at,
                now,
            ],
        )?;
        query_effect(&conn, effect_id)?.context("finished effect disappeared")
    }

    fn mark_running_effects_indeterminate(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let now = Utc::now().to_rfc3339();
        let updated = conn.execute(
            r#"
            UPDATE effect_journal
            SET status = ?1,
                error = COALESCE(error, 'process exited before the effect outcome was persisted'),
                updated_at = ?2
            WHERE status = ?3
            "#,
            params![
                EffectStatus::Indeterminate.as_str(),
                now,
                EffectStatus::Running.as_str(),
            ],
        )?;
        Ok(updated)
    }

    fn insert_terminal_history(
        &self,
        history: TerminalCommandHistory,
    ) -> anyhow::Result<TerminalCommandHistory> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let seq_start = i64::try_from(history.seq_start)
            .context("terminal seq_start exceeds sqlite INTEGER")?;
        let seq_end =
            i64::try_from(history.seq_end).context("terminal seq_end exceeds sqlite INTEGER")?;
        conn.execute(
            r#"
            INSERT INTO terminal_history (
                command_id, thread_id, seq_start, seq_end, command, cwd, stdout,
                stderr, exit_code, status, message, started_at, completed_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(command_id) DO UPDATE SET
                seq_start = excluded.seq_start,
                seq_end = excluded.seq_end,
                command = excluded.command,
                cwd = excluded.cwd,
                stdout = excluded.stdout,
                stderr = excluded.stderr,
                exit_code = excluded.exit_code,
                status = excluded.status,
                message = excluded.message,
                started_at = excluded.started_at,
                completed_at = excluded.completed_at
            "#,
            params![
                history.command_id.to_string(),
                history.thread_id.to_string(),
                seq_start,
                seq_end,
                &history.command,
                history.cwd.as_ref().map(|path| path.display().to_string()),
                &history.stdout,
                &history.stderr,
                history.exit_code,
                history.status.as_str(),
                &history.message,
                history.started_at.to_rfc3339(),
                history.completed_at.to_rfc3339(),
            ],
        )?;
        touch_thread(&conn, history.thread_id)?;
        Ok(history)
    }

    fn list_terminal_history(
        &self,
        thread_id: Uuid,
        after_seq: Option<u64>,
    ) -> anyhow::Result<Vec<TerminalCommandHistory>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let after_seq = i64::try_from(after_seq.unwrap_or(0))
            .context("terminal after_seq exceeds sqlite INTEGER")?;
        let mut stmt = conn.prepare(
            r#"
            SELECT command_id, thread_id, seq_start, seq_end, command, cwd, stdout,
                   stderr, exit_code, status, message, started_at, completed_at
            FROM terminal_history
            WHERE thread_id = ?1 AND seq_end > ?2
            ORDER BY seq_start ASC
            "#,
        )?;
        let rows = stmt.query_map(
            params![thread_id.to_string(), after_seq],
            map_terminal_history,
        )?;
        collect_rows(rows)
    }

    fn latest_terminal_history_seq(&self, thread_id: Uuid) -> anyhow::Result<u64> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq_end), 0) FROM terminal_history WHERE thread_id = ?1",
            params![thread_id.to_string()],
            |row| row.get(0),
        )?;
        parse_u64(seq, 0).map_err(Into::into)
    }

    fn insert_artifact(&self, artifact: Artifact) -> anyhow::Result<Artifact> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let (storage_kind, path, inline_content) = match &artifact.storage {
            ArtifactStorage::Inline { content } => ("inline", None, Some(content.as_str())),
            ArtifactStorage::Path { path } => ("path", Some(path.display().to_string()), None),
        };
        let metadata_json = serde_json::to_string(&artifact.metadata)?;
        let bytes =
            i64::try_from(artifact.bytes).context("artifact bytes exceed sqlite INTEGER")?;
        conn.execute(
            r#"
            INSERT INTO artifacts (
                id, thread_id, kind, content_type, storage_kind, path, inline_content,
                bytes, metadata_json, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                artifact.id.to_string(),
                artifact.thread_id.to_string(),
                &artifact.kind,
                &artifact.content_type,
                storage_kind,
                path,
                inline_content,
                bytes,
                metadata_json,
                artifact.created_at.to_rfc3339(),
            ],
        )?;
        touch_thread(&conn, artifact.thread_id)?;
        Ok(artifact)
    }

    fn list_artifacts(&self, thread_id: Uuid) -> anyhow::Result<Vec<ArtifactMetadata>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT id, thread_id, kind, content_type, storage_kind, path,
                   bytes, metadata_json, created_at
            FROM artifacts
            WHERE thread_id = ?1
            ORDER BY created_at ASC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], map_artifact_metadata)?;
        collect_rows(rows)
    }

    fn get_artifact(&self, thread_id: Uuid, artifact_id: Uuid) -> anyhow::Result<Option<Artifact>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let artifact = conn
            .query_row(
                r#"
                SELECT id, thread_id, kind, content_type, storage_kind, path, inline_content,
                       bytes, metadata_json, created_at
                FROM artifacts
                WHERE thread_id = ?1 AND id = ?2
                "#,
                params![thread_id.to_string(), artifact_id.to_string()],
                map_artifact,
            )
            .optional()?;
        Ok(artifact)
    }

    fn save_provider_conversation_state(
        &self,
        state: &ProviderConversationState,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        provider_conversation_state::save(&conn, state)
    }

    fn get_provider_conversation_state(
        &self,
        thread_id: Uuid,
        agent_path: &str,
    ) -> anyhow::Result<Option<ProviderConversationState>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        provider_conversation_state::load(&conn, thread_id, agent_path)
    }

    fn take_provider_conversation_state(
        &self,
        thread_id: Uuid,
        agent_path: &str,
    ) -> anyhow::Result<Option<ProviderConversationState>> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        provider_conversation_state::take(&mut conn, thread_id, agent_path)
    }

    fn clear_provider_conversation_state(
        &self,
        thread_id: Uuid,
        agent_path: &str,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        provider_conversation_state::clear(&conn, thread_id, agent_path)
    }

    fn insert_approval(&self, approval: Approval) -> anyhow::Result<Approval> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO approvals (
                approval_id, thread_id, action, reason, status, created_at, decided_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                approval.approval_id.to_string(),
                approval.thread_id.to_string(),
                &approval.action,
                &approval.reason,
                approval.status.as_str(),
                approval.created_at.to_rfc3339(),
                approval.decided_at.as_ref().map(DateTime::to_rfc3339),
            ],
        )?;
        touch_thread(&conn, approval.thread_id)?;
        Ok(approval)
    }

    fn get_approval(&self, approval_id: Uuid) -> anyhow::Result<Option<Approval>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let approval = conn
            .query_row(
                r#"
                SELECT approval_id, thread_id, action, reason, status, created_at, decided_at
                FROM approvals
                WHERE approval_id = ?1
                "#,
                params![approval_id.to_string()],
                map_approval,
            )
            .optional()?;
        Ok(approval)
    }

    fn list_approvals(
        &self,
        thread_id: Uuid,
        status: Option<ApprovalStatus>,
    ) -> anyhow::Result<Vec<Approval>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        if let Some(status) = status {
            let mut stmt = conn.prepare(
                r#"
                SELECT approval_id, thread_id, action, reason, status, created_at, decided_at
                FROM approvals
                WHERE thread_id = ?1 AND status = ?2
                ORDER BY created_at ASC
                "#,
            )?;
            let rows = stmt.query_map(
                params![thread_id.to_string(), status.as_str()],
                map_approval,
            )?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                r#"
                SELECT approval_id, thread_id, action, reason, status, created_at, decided_at
                FROM approvals
                WHERE thread_id = ?1
                ORDER BY created_at ASC
                "#,
            )?;
            let rows = stmt.query_map(params![thread_id.to_string()], map_approval)?;
            collect_rows(rows)
        }
    }

    fn update_approval_status(
        &self,
        approval_id: Uuid,
        status: ApprovalStatus,
    ) -> anyhow::Result<Option<Approval>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let decided_at = match status {
            ApprovalStatus::Pending => None,
            ApprovalStatus::Approved | ApprovalStatus::Denied => Some(Utc::now()),
        };
        let updated = conn.execute(
            r#"
            UPDATE approvals
            SET status = ?1, decided_at = ?2
            WHERE approval_id = ?3 AND status = ?4
            "#,
            params![
                status.as_str(),
                decided_at.as_ref().map(DateTime::to_rfc3339),
                approval_id.to_string(),
                ApprovalStatus::Pending.as_str(),
            ],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        let approval = conn.query_row(
            r#"
            SELECT approval_id, thread_id, action, reason, status, created_at, decided_at
            FROM approvals
            WHERE approval_id = ?1
            "#,
            params![approval_id.to_string()],
            map_approval,
        )?;
        touch_thread(&conn, approval.thread_id)?;
        Ok(Some(approval))
    }

    fn put_approval_continuation(
        &self,
        approval_id: Uuid,
        thread_id: Uuid,
        continuation: Value,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO approval_continuations
                (approval_id, thread_id, continuation_json, created_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(approval_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                continuation_json = excluded.continuation_json,
                created_at = excluded.created_at
            "#,
            params![
                approval_id.to_string(),
                thread_id.to_string(),
                serde_json::to_string(&continuation)?,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    fn get_approval_continuation(
        &self,
        approval_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<Value>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let continuation = conn
            .query_row(
                r#"
                SELECT continuation_json
                FROM approval_continuations
                WHERE approval_id = ?1 AND thread_id = ?2
                "#,
                params![approval_id.to_string(), thread_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        continuation
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    fn delete_approval_continuation(
        &self,
        approval_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            "DELETE FROM approval_continuations WHERE approval_id = ?1 AND thread_id = ?2",
            params![approval_id.to_string(), thread_id.to_string()],
        )?;
        Ok(())
    }

    fn put_user_input_request(
        &self,
        thread_id: Uuid,
        request: &UserInputRequest,
        continuation: Value,
    ) -> anyhow::Result<UserInputRecord> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let created_at = Utc::now();
        conn.execute(
            r#"
            INSERT INTO user_input_requests (
                request_id, thread_id, request_json, status, response_json,
                continuation_json, created_at, answered_at
            ) VALUES (?1, ?2, ?3, 'pending', NULL, ?4, ?5, NULL)
            ON CONFLICT(request_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                request_json = excluded.request_json,
                status = 'pending',
                response_json = NULL,
                continuation_json = excluded.continuation_json,
                created_at = excluded.created_at,
                answered_at = NULL
            "#,
            params![
                request.request_id.to_string(),
                thread_id.to_string(),
                serde_json::to_string(request)?,
                serde_json::to_string(&continuation)?,
                created_at.to_rfc3339(),
            ],
        )?;
        touch_thread(&conn, thread_id)?;
        Ok(UserInputRecord {
            thread_id,
            request: request.clone(),
            status: UserInputStatus::Pending,
            response: None,
            created_at,
            answered_at: None,
        })
    }

    fn get_user_input_request(&self, request_id: Uuid) -> anyhow::Result<Option<UserInputRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            r#"
            SELECT request_id, thread_id, request_json, status, response_json,
                   created_at, answered_at
            FROM user_input_requests
            WHERE request_id = ?1
            "#,
            params![request_id.to_string()],
            map_user_input_record,
        )
        .optional()
        .map_err(Into::into)
    }

    fn list_user_input_requests(
        &self,
        thread_id: Uuid,
        status: Option<UserInputStatus>,
    ) -> anyhow::Result<Vec<UserInputRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let select = r#"
            SELECT request_id, thread_id, request_json, status, response_json,
                   created_at, answered_at
            FROM user_input_requests
        "#;
        let records = if let Some(status) = status {
            let mut stmt = conn.prepare(&format!(
                "{select} WHERE thread_id = ?1 AND status = ?2 ORDER BY created_at ASC"
            ))?;
            let records = collect_rows(stmt.query_map(
                params![thread_id.to_string(), status.as_str()],
                map_user_input_record,
            )?)?;
            records
        } else {
            let mut stmt = conn.prepare(&format!(
                "{select} WHERE thread_id = ?1 ORDER BY created_at ASC"
            ))?;
            let records = collect_rows(
                stmt.query_map(params![thread_id.to_string()], map_user_input_record)?,
            )?;
            records
        };
        Ok(records)
    }

    fn get_user_input_continuation(
        &self,
        request_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<Value>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let continuation = conn
            .query_row(
                r#"
                SELECT continuation_json
                FROM user_input_requests
                WHERE request_id = ?1 AND thread_id = ?2 AND status = 'pending'
                "#,
                params![request_id.to_string(), thread_id.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        continuation
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    fn resolve_user_input_request(
        &self,
        request_id: Uuid,
        thread_id: Uuid,
        response: &UserInputResponse,
    ) -> anyhow::Result<Option<UserInputRecord>> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let answered_at = Utc::now();
        let changed = tx.execute(
            r#"
            UPDATE user_input_requests
            SET status = 'answered', response_json = ?1, continuation_json = NULL,
                answered_at = ?2
            WHERE request_id = ?3 AND thread_id = ?4 AND status = 'pending'
            "#,
            params![
                serde_json::to_string(response)?,
                answered_at.to_rfc3339(),
                request_id.to_string(),
                thread_id.to_string(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        touch_thread(&tx, thread_id)?;
        tx.commit()?;
        drop(conn);
        self.get_user_input_request(request_id)
    }

    fn put_turn_checkpoint(
        &self,
        turn_id: Uuid,
        thread_id: Uuid,
        wait_kind: &str,
        checkpoint: Value,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(wait_kind, "approval" | "user_input" | "external_action"),
            "unsupported turn checkpoint kind: {wait_kind}"
        );
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let now = Utc::now().to_rfc3339();
        conn.execute(
            r#"
            INSERT INTO turn_checkpoints (
                turn_id, thread_id, wait_kind, checkpoint_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
            ON CONFLICT(turn_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                wait_kind = excluded.wait_kind,
                checkpoint_json = excluded.checkpoint_json,
                updated_at = excluded.updated_at
            "#,
            params![
                turn_id.to_string(),
                thread_id.to_string(),
                wait_kind,
                serde_json::to_string(&checkpoint)?,
                now,
            ],
        )?;
        Ok(())
    }

    fn get_turn_checkpoint(
        &self,
        turn_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<(String, Value)>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let row = conn
            .query_row(
                r#"
                SELECT wait_kind, checkpoint_json
                FROM turn_checkpoints
                WHERE turn_id = ?1 AND thread_id = ?2
                "#,
                params![turn_id.to_string(), thread_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        row.map(|(wait_kind, checkpoint)| {
            Ok((wait_kind, serde_json::from_str::<Value>(&checkpoint)?))
        })
        .transpose()
    }

    fn delete_turn_checkpoint(&self, turn_id: Uuid, thread_id: Uuid) -> anyhow::Result<bool> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        Ok(conn.execute(
            "DELETE FROM turn_checkpoints WHERE turn_id = ?1 AND thread_id = ?2",
            params![turn_id.to_string(), thread_id.to_string()],
        )? > 0)
    }

    fn put_turn_checkpoint_blob(&self, kind: &str, payload: Value) -> anyhow::Result<String> {
        anyhow::ensure!(
            matches!(kind, "conversation" | "model_context" | "tool_catalog"),
            "unsupported turn checkpoint blob kind: {kind}"
        );
        let payload_json = serde_json::to_string(&payload)?;
        // Domain-separate checkpoint blobs by kind. Conversation and tool
        // catalogs can both legitimately serialize to `[]`; sharing a raw
        // payload hash would let one blob silently acquire the other's type.
        let hash_input = format!("{kind}\0{payload_json}");
        let content_hash = crate::model_context::content_fingerprint(hash_input.as_bytes());
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT OR IGNORE INTO turn_checkpoint_blobs (
                content_hash, kind, payload_json, created_at
            ) VALUES (?1, ?2, ?3, ?4)
            "#,
            params![content_hash, kind, payload_json, Utc::now().to_rfc3339(),],
        )?;
        Ok(content_hash)
    }

    fn get_turn_checkpoint_blob(&self, content_hash: &str) -> anyhow::Result<Option<Value>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let payload = conn
            .query_row(
                "SELECT payload_json FROM turn_checkpoint_blobs WHERE content_hash = ?1",
                params![content_hash],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
            .transpose()
    }
}

#[cfg(test)]
mod tests;
