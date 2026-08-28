use super::*;
use crate::completion_runtime::CompletionDisposition;
use crate::work_form::{WorkForm, WorkItem, WorkItemStatus, WorkScope};
use chrono::Utc;

const MAX_ITEMS: usize = 20;
const MAX_ID_CHARS: usize = 100;
const MAX_TEXT_CHARS: usize = 300;
const MAX_NOTE_CHARS: usize = 1_000;
const MAX_REASON_CHARS: usize = 2_000;
const MAX_LIST_ITEMS: usize = 20;
const TURN_PLAN_OBJECTIVE: &str = "Current task plan";

/// A complete snapshot of one visible checklist item.
///
/// Ordinary Default-mode plans only need `step` and `status`. The remaining
/// fields preserve the richer, server-owned Goal WorkForm contract without
/// changing the snapshot semantics of the model-facing tool.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlanItemInput {
    /// Optional stable ID for dependency-aware Goal work. Ordinary plans may omit it.
    #[serde(default)]
    id: Option<String>,
    /// Concise executable step.
    step: String,
    /// Current status of this step.
    status: WorkItemStatus,
    #[serde(default)]
    completion_disposition: CompletionDisposition,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    #[schemars(length(max = 20))]
    acceptance: Vec<String>,
    #[serde(default)]
    evidence_refs: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlanInput {
    /// Optional concise explanation for replacing the displayed plan snapshot.
    #[serde(default)]
    explanation: Option<String>,
    /// The complete current plan. Every call replaces the previous snapshot atomically.
    #[schemars(length(min = 1, max = 20))]
    plan: Vec<PlanItemInput>,
}

pub struct UpdatePlanTool;

#[async_trait]
impl TypedTool for UpdatePlanTool {
    type Input = UpdatePlanInput;

    fn name(&self) -> &str {
        "update_plan"
    }

    fn description(&self) -> &str {
        "Publish the complete current task-plan snapshot. Each call atomically creates or replaces the checklist, so no prior plan is required. Use concise step/status entries for ordinary work; IDs, dependencies, notes, acceptance criteria, and evidence are optional Goal-work details."
    }

    fn validate_context(&self, ctx: &ToolInvocationContext) -> anyhow::Result<()> {
        anyhow::ensure!(
            ctx.agent_depth == 0,
            "only the parent agent may update the shared WorkForm"
        );
        anyhow::ensure!(
            ctx.collaboration_mode != CollaborationMode::Plan,
            "update_plan is an execution checklist tool and is not allowed in Plan mode"
        );
        Ok(())
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let (thread_id, scope) = work_scope(&ctx)?;
        let existing = ctx
            .current_work_form
            .filter(|form| form.scope == scope && form.thread_id == thread_id);
        let objective = match (&existing, scope) {
            (Some(form), _) => form.objective.clone(),
            (None, WorkScope::Turn(_)) => TURN_PLAN_OBJECTIVE.to_string(),
            (None, WorkScope::Goal(_)) => {
                anyhow::bail!("the active Goal is missing its server-owned WorkForm")
            }
        };
        let items = input
            .plan
            .into_iter()
            .enumerate()
            .map(|(index, item)| plan_item(item, index))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let mut form = WorkForm::new(thread_id, scope, objective, items);
        if let Some(existing) = existing {
            form.constraints = existing.constraints;
            form.acceptance = existing.acceptance;
            form.created_at = existing.created_at;
            form.revision = existing
                .revision
                .checked_add(1)
                .context("WorkForm revision overflow")?;
        } else {
            form.revision = 1;
        }
        form.change_reason = input
            .explanation
            .map(|reason| validate_text("explanation", reason, MAX_REASON_CHARS))
            .transpose()?;
        form.updated_at = Utc::now();
        form.validate()?;
        work_form_result(call_id, form)
    }
}

impl_typed_tool!(UpdatePlanTool);

fn work_scope(ctx: &ToolInvocationContext) -> anyhow::Result<(Uuid, WorkScope)> {
    let thread_id = ctx
        .thread_id
        .context("WorkForm tools require a thread id")?;
    let scope = ctx
        .goal_id
        .map(WorkScope::Goal)
        .or_else(|| ctx.agent_turn_id.map(WorkScope::Turn))
        .context("WorkForm tools require a Goal or Turn scope")?;
    Ok((thread_id, scope))
}

fn plan_item(input: PlanItemInput, index: usize) -> anyhow::Result<WorkItem> {
    Ok(WorkItem {
        id: validate_text(
            "plan.id",
            input.id.unwrap_or_else(|| format!("step_{}", index + 1)),
            MAX_ID_CHARS,
        )?,
        title: validate_text("plan.step", input.step, MAX_TEXT_CHARS)?,
        status: input.status,
        completion_disposition: input.completion_disposition,
        depends_on: validate_ids("plan.depends_on", input.depends_on)?,
        note: input
            .note
            .map(|note| validate_text("plan.note", note, MAX_NOTE_CHARS))
            .transpose()?,
        acceptance: validate_list("plan.acceptance", input.acceptance)?,
        evidence_refs: validate_list("plan.evidence_refs", input.evidence_refs)?,
    })
}

fn validate_text(field: &str, value: String, max_chars: usize) -> anyhow::Result<String> {
    let value = value.trim().to_string();
    anyhow::ensure!(!value.is_empty(), "{field} cannot be empty");
    anyhow::ensure!(
        value.chars().count() <= max_chars,
        "{field} exceeds the {max_chars} character limit"
    );
    Ok(value)
}

fn validate_list(field: &str, values: Vec<String>) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(
        values.len() <= MAX_LIST_ITEMS,
        "{field} may contain at most {MAX_LIST_ITEMS} values"
    );
    let mut unique = HashSet::new();
    values
        .into_iter()
        .map(|value| {
            let value = validate_text(field, value, MAX_TEXT_CHARS)?;
            anyhow::ensure!(
                unique.insert(value.to_lowercase()),
                "{field} contains a duplicate value: {value}"
            );
            Ok(value)
        })
        .collect()
}

fn validate_ids(field: &str, values: Vec<String>) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(
        values.len() <= MAX_ITEMS,
        "{field} may contain at most {MAX_ITEMS} ids"
    );
    let mut unique = HashSet::new();
    values
        .into_iter()
        .map(|value| {
            let value = validate_text(field, value, MAX_ID_CHARS)?;
            anyhow::ensure!(
                unique.insert(value.clone()),
                "{field} contains a duplicate id: {value}"
            );
            Ok(value)
        })
        .collect()
}

fn work_form_result(call_id: Uuid, form: WorkForm) -> anyhow::Result<ToolResult> {
    let next = form.next_runnable_item().cloned();
    let next_index = next
        .as_ref()
        .and_then(|next| form.items.iter().position(|item| item.id == next.id))
        .map(|index| index + 1);
    let value = serde_json::to_value(&form)?;
    Ok(ToolResult {
        call_id,
        output: "Plan updated".to_string(),
        content: vec![ModelContentPart::json(value.clone())],
        metadata: json!({
            "toolName": "update_plan",
            "workForm": value,
            "formId": form.id,
            "revision": form.revision,
            "status": form.status,
            "completed": form.completed_items(),
            "resolved": form.resolved_items(),
            "total": form.items.len(),
            "nextRunnableItem": next,
            "currentItemIndex": next_index,
            "success": true
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{BasicPolicyEngine, PermissionMode};

    fn context(thread_id: Uuid, turn_id: Uuid) -> ToolInvocationContext {
        let workspace = std::env::current_dir().expect("workspace");
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace.clone(),
            PermissionMode::FullAccess,
        ));
        let mut context = ToolInvocationContext::local(workspace, policy);
        context.thread_id = Some(thread_id);
        context.agent_turn_id = Some(turn_id);
        context
    }

    async fn publish(context: ToolInvocationContext, input: Value) -> anyhow::Result<ToolResult> {
        UpdatePlanTool
            .execute(ToolCall::new("update_plan", input), context)
            .await
    }

    #[tokio::test]
    async fn first_snapshot_creates_a_turn_work_form() {
        let turn_id = Uuid::new_v4();
        let result = publish(
            context(Uuid::new_v4(), turn_id),
            json!({
                "plan": [
                    { "step": "Inspect", "status": "completed" },
                    { "step": "Implement", "status": "in_progress" }
                ]
            }),
        )
        .await
        .expect("publish first snapshot");

        let form: WorkForm =
            serde_json::from_value(result.metadata["workForm"].clone()).expect("WorkForm");
        assert_eq!(result.output, "Plan updated");
        assert_eq!(form.scope, WorkScope::Turn(turn_id));
        assert_eq!(form.revision, 1);
        assert_eq!(form.items[0].id, "step_1");
        assert_eq!(form.items[1].status, WorkItemStatus::InProgress);
    }

    #[tokio::test]
    async fn later_snapshot_replaces_the_complete_plan() {
        let thread_id = Uuid::new_v4();
        let turn_id = Uuid::new_v4();
        let created = publish(
            context(thread_id, turn_id),
            json!({ "plan": [{ "step": "Old step", "status": "in_progress" }] }),
        )
        .await
        .expect("publish first snapshot");
        let original: WorkForm =
            serde_json::from_value(created.metadata["workForm"].clone()).expect("WorkForm");

        let mut next_context = context(thread_id, turn_id);
        next_context.current_work_form = Some(original.clone());
        let replaced = publish(
            next_context,
            json!({
                "explanation": "Evidence changed the approach",
                "plan": [{ "step": "New step", "status": "completed" }]
            }),
        )
        .await
        .expect("replace snapshot");
        let current: WorkForm =
            serde_json::from_value(replaced.metadata["workForm"].clone()).expect("WorkForm");

        assert_eq!(current.revision, 2);
        assert_eq!(current.created_at, original.created_at);
        assert_eq!(current.items.len(), 1);
        assert_eq!(current.items[0].title, "New step");
        assert_eq!(current.status, crate::work_form::WorkFormStatus::Completed);
    }

    #[tokio::test]
    async fn a_new_turn_can_republish_an_interrupted_plan_without_prior_state() {
        let new_turn_id = Uuid::new_v4();
        let result = publish(
            context(Uuid::new_v4(), new_turn_id),
            json!({
                "plan": [
                    { "step": "Inspect", "status": "completed" },
                    { "step": "Continue implementation", "status": "in_progress" }
                ]
            }),
        )
        .await
        .expect("republish complete snapshot in a new Turn");
        let form: WorkForm =
            serde_json::from_value(result.metadata["workForm"].clone()).expect("WorkForm");

        assert_eq!(form.scope, WorkScope::Turn(new_turn_id));
        assert_eq!(form.revision, 1);
        assert_eq!(form.items[1].status, WorkItemStatus::InProgress);
    }

    #[tokio::test]
    async fn goal_snapshot_preserves_the_server_owned_definition() {
        let thread_id = Uuid::new_v4();
        let goal_id = Uuid::new_v4();
        let mut existing = WorkForm::empty_goal(thread_id, goal_id, "Ship release".to_string());
        existing.constraints = vec!["Keep compatibility".to_string()];
        existing.acceptance = vec!["Tests pass".to_string()];
        let mut goal_context = context(thread_id, Uuid::new_v4());
        goal_context.goal_id = Some(goal_id);
        goal_context.current_work_form = Some(existing);

        let result = publish(
            goal_context,
            json!({
                "plan": [{
                    "id": "verify",
                    "step": "Verify release",
                    "status": "completed",
                    "acceptance": ["Release checks pass"],
                    "evidence_refs": ["test:release"]
                }]
            }),
        )
        .await
        .expect("publish Goal snapshot");
        let form: WorkForm =
            serde_json::from_value(result.metadata["workForm"].clone()).expect("WorkForm");

        assert_eq!(form.objective, "Ship release");
        assert_eq!(form.constraints, vec!["Keep compatibility"]);
        assert_eq!(form.acceptance, vec!["Tests pass"]);
        assert_eq!(form.status, crate::work_form::WorkFormStatus::Completed);
    }

    #[test]
    fn schema_accepts_snapshots_and_rejects_the_removed_patch_protocol() {
        assert!(UpdatePlanTool
            .input_error(&json!({
                "plan": [{ "step": "Inspect", "status": "in_progress" }]
            }))
            .is_none());
        assert!(UpdatePlanTool
            .input_error(&json!({
                "items": [{ "id": "inspect", "status": "completed" }]
            }))
            .is_some());
        let schema = UpdatePlanTool.schema().to_string();
        assert!(schema.contains("\"plan\""));
        assert!(!schema.contains("\"append\""));
        assert!(!schema.contains("\"remove\""));
    }

    #[test]
    fn execution_checklist_tool_rejects_plan_mode() {
        let mut plan_context = context(Uuid::new_v4(), Uuid::new_v4());
        plan_context.collaboration_mode = CollaborationMode::Plan;

        let error = <UpdatePlanTool as TypedTool>::validate_context(&UpdatePlanTool, &plan_context)
            .expect_err("update_plan must reject Plan mode");
        assert!(error.to_string().contains("execution checklist tool"));
    }
}
