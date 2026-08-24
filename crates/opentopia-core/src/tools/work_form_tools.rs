use super::*;
use crate::completion_runtime::CompletionDisposition;
use crate::work_form::{WorkForm, WorkFormStatus, WorkItem, WorkItemStatus, WorkScope};
use chrono::Utc;

const MAX_ITEMS: usize = 20;
const MAX_ID_CHARS: usize = 100;
const MAX_TEXT_CHARS: usize = 300;
const MAX_NOTE_CHARS: usize = 1_000;
const MAX_REASON_CHARS: usize = 2_000;
const MAX_LIST_ITEMS: usize = 20;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetWorkFormInput {
    /// Required for a new ordinary Turn form. Goal forms inherit the server objective.
    #[serde(default)]
    objective: Option<String>,
    /// Optional concise explanation for replacing the plan.
    #[serde(default)]
    explanation: Option<String>,
    #[serde(default)]
    constraints: Vec<String>,
    #[serde(default)]
    acceptance: Vec<String>,
    #[schemars(length(min = 1, max = 20))]
    items: Vec<NewWorkItemInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NewWorkItemInput {
    /// Optional stable ID. Omit to generate step_1, step_2, and so on.
    #[serde(default)]
    id: Option<String>,
    /// Concise executable step.
    step: String,
    #[serde(default)]
    status: Option<WorkItemStatus>,
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

pub struct SetPlanTool;

#[async_trait]
impl TypedTool for SetPlanTool {
    type Input = SetWorkFormInput;

    fn name(&self) -> &str {
        "set_plan"
    }

    fn description(&self) -> &str {
        "Create or replace the current task plan as external memory. Plans are optional for simple or focused work. Provide an objective and concise executable items with a step; item IDs are optional and otherwise generated as step_1, step_2, and so on. Status, dependencies, notes, acceptance criteria, and evidence are optional."
    }

    fn validate_context(&self, ctx: &ToolInvocationContext) -> anyhow::Result<()> {
        anyhow::ensure!(
            ctx.agent_depth == 0,
            "only the parent agent may set the shared WorkForm"
        );
        anyhow::ensure!(
            ctx.collaboration_mode != CollaborationMode::Plan,
            "set_plan is an execution checklist tool and is not allowed in Plan mode"
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
        let existing = ctx.current_work_form.as_ref();
        let observed_revision = existing
            .filter(|form| form.scope == scope)
            .map(|form| form.revision)
            .unwrap_or(0);
        let objective = match input.objective {
            Some(objective) => validate_text("objective", objective, MAX_TEXT_CHARS)?,
            None => existing
                .map(|form| form.objective.clone())
                .context("a new Turn WorkForm requires objective")?,
        };
        let mut items = Vec::with_capacity(input.items.len());
        for (index, item) in input.items.into_iter().enumerate() {
            let id = item.id.unwrap_or_else(|| format!("step_{}", index + 1));
            let status = item.status.unwrap_or(WorkItemStatus::Pending);
            items.push(WorkItem {
                id: validate_text("item.id", id, MAX_ID_CHARS)?,
                title: validate_text("item.step", item.step, MAX_TEXT_CHARS)?,
                status,
                completion_disposition: item.completion_disposition,
                depends_on: validate_ids("item.depends_on", item.depends_on)?,
                note: item
                    .note
                    .map(|note| validate_text("item.note", note, MAX_NOTE_CHARS))
                    .transpose()?,
                acceptance: validate_list("item.acceptance", item.acceptance)?,
                evidence_refs: validate_list("item.evidence_refs", item.evidence_refs)?,
            });
        }
        let mut form = WorkForm::new(thread_id, scope, objective, items);
        form.constraints = validate_list("constraints", input.constraints)?;
        form.acceptance = validate_list("acceptance", input.acceptance)?;
        form.change_reason = input
            .explanation
            .map(|reason| validate_text("explanation", reason, MAX_REASON_CHARS))
            .transpose()?;
        form.revision = observed_revision
            .checked_add(1)
            .context("WorkForm revision overflow")?;
        if let Some(existing) = existing {
            form.created_at = existing.created_at;
        }
        form.updated_at = Utc::now();
        form.validate()?;
        work_form_result(call_id, "set_plan", form, None)
    }
}

impl_typed_tool!(SetPlanTool);

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateWorkItemInput {
    #[serde(default)]
    step: Option<String>,
    #[serde(default)]
    status: Option<WorkItemStatus>,
    #[serde(default)]
    completion_disposition: Option<CompletionDisposition>,
    #[serde(default)]
    depends_on: Option<Vec<String>>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    acceptance: Option<Vec<String>>,
    #[serde(default)]
    evidence_refs: Option<Vec<String>>,
}

impl UpdateWorkItemInput {
    fn is_empty(&self) -> bool {
        self.step.is_none()
            && self.status.is_none()
            && self.completion_disposition.is_none()
            && self.depends_on.is_none()
            && self.note.is_none()
            && self.acceptance.is_none()
            && self.evidence_refs.is_none()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CompactWorkItemUpdateInput {
    id: String,
    #[serde(flatten)]
    updates: UpdateWorkItemInput,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateWorkFormInput {
    #[serde(default)]
    objective: Option<String>,
    #[serde(default)]
    constraints: Option<Vec<String>>,
    #[serde(default)]
    acceptance: Option<Vec<String>>,
    #[serde(default)]
    status: Option<WorkFormStatus>,
}

impl UpdateWorkFormInput {
    fn is_empty(&self) -> bool {
        self.objective.is_none()
            && self.constraints.is_none()
            && self.acceptance.is_none()
            && self.status.is_none()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlanInput {
    /// Update several existing items atomically.
    #[serde(default)]
    items: Vec<CompactWorkItemUpdateInput>,
    /// Append new items atomically.
    #[serde(default)]
    append: Vec<NewWorkItemInput>,
    /// Remove item IDs after applying updates.
    #[serde(default)]
    remove: Vec<String>,
    /// Optional concise explanation for this update.
    #[serde(default)]
    explanation: Option<String>,
    #[serde(default)]
    form: Option<UpdateWorkFormInput>,
}

pub struct UpdatePlanTool;

#[async_trait]
impl TypedTool for UpdatePlanTool {
    type Input = UpdatePlanInput;

    fn name(&self) -> &str {
        "update_plan"
    }

    fn description(&self) -> &str {
        "Update the current task plan atomically after a plan has been created. Batch existing item patches in items, new steps in append, removals in remove, and optional form changes. The runtime owns revision control; status, notes, acceptance criteria, and evidence are optional for ordinary task plans."
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
        let (_, scope) = work_scope(&ctx)?;
        let mut work_form = ctx
            .current_work_form
            .context("no WorkForm exists; create one with set_plan")?;
        anyhow::ensure!(
            work_form.scope == scope,
            "current WorkForm belongs to another scope"
        );
        let explicit_form_status = input.form.as_ref().and_then(|form| form.status);
        let changed_item_id = apply_plan_update(
            &mut work_form,
            input.items,
            input.append,
            input.remove,
            input.form,
        )?;

        if explicit_form_status.is_none() {
            work_form.recalculate_status();
        }
        work_form.revision = work_form
            .revision
            .checked_add(1)
            .context("WorkForm revision overflow")?;
        work_form.change_reason = input
            .explanation
            .map(|reason| validate_text("explanation", reason, MAX_REASON_CHARS))
            .transpose()?;
        work_form.updated_at = Utc::now();
        work_form.validate()?;
        work_form_result(call_id, "update_plan", work_form, changed_item_id)
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

fn validate_item(input: NewWorkItemInput, generated_id: String) -> anyhow::Result<WorkItem> {
    Ok(WorkItem {
        id: validate_text("item.id", input.id.unwrap_or(generated_id), MAX_ID_CHARS)?,
        title: validate_text("item.step", input.step, MAX_TEXT_CHARS)?,
        status: input.status.unwrap_or(WorkItemStatus::Pending),
        completion_disposition: input.completion_disposition,
        depends_on: validate_ids("item.depends_on", input.depends_on)?,
        note: input
            .note
            .map(|note| validate_text("item.note", note, MAX_NOTE_CHARS))
            .transpose()?,
        acceptance: validate_list("item.acceptance", input.acceptance)?,
        evidence_refs: validate_list("item.evidence_refs", input.evidence_refs)?,
    })
}

fn apply_plan_update(
    work_form: &mut WorkForm,
    items: Vec<CompactWorkItemUpdateInput>,
    append: Vec<NewWorkItemInput>,
    remove: Vec<String>,
    form: Option<UpdateWorkFormInput>,
) -> anyhow::Result<Option<String>> {
    anyhow::ensure!(
        !items.is_empty() || !append.is_empty() || !remove.is_empty() || form.is_some(),
        "update_plan requires at least one item, append, remove, or form change"
    );
    let mut changed_ids = Vec::new();

    for input in append {
        anyhow::ensure!(
            work_form.items.len() < MAX_ITEMS,
            "WorkForm may contain at most {MAX_ITEMS} items"
        );
        let item = validate_item(input, next_generated_item_id(work_form))?;
        anyhow::ensure!(
            !work_form.items.iter().any(|current| current.id == item.id),
            "WorkForm already contains item id: {}",
            item.id
        );
        changed_ids.push(item.id.clone());
        work_form.items.push(item);
    }

    for patch in items {
        let item_id = validate_text("items.id", patch.id, MAX_ID_CHARS)?;
        anyhow::ensure!(
            !patch.updates.is_empty(),
            "item {item_id} requires at least one changed field"
        );
        let item = work_form
            .items
            .iter_mut()
            .find(|item| item.id == item_id)
            .with_context(|| format!("WorkForm does not contain item id: {item_id}"))?;
        apply_item_updates(item, patch.updates)?;
        changed_ids.push(item_id);
    }

    let remove = validate_ids("remove", remove)?;
    if !remove.is_empty() {
        let remove_set = remove.iter().cloned().collect::<HashSet<_>>();
        let dependents = work_form
            .items
            .iter()
            .filter(|item| {
                !remove_set.contains(&item.id)
                    && item
                        .depends_on
                        .iter()
                        .any(|dependency| remove_set.contains(dependency))
            })
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        anyhow::ensure!(
            dependents.is_empty(),
            "cannot remove items; still required by: {}",
            dependents.join(", ")
        );
        let before = work_form.items.len();
        work_form
            .items
            .retain(|item| !remove_set.contains(&item.id));
        anyhow::ensure!(
            before.saturating_sub(work_form.items.len()) == remove_set.len(),
            "remove contains an unknown WorkForm item id"
        );
        changed_ids.extend(remove);
    }

    if let Some(update) = form {
        apply_form_updates(work_form, update)?;
    }

    changed_ids.sort();
    changed_ids.dedup();
    Ok((changed_ids.len() == 1).then(|| changed_ids.remove(0)))
}

fn next_generated_item_id(work_form: &WorkForm) -> String {
    (1..=MAX_ITEMS + 1)
        .map(|index| format!("step_{index}"))
        .find(|candidate| !work_form.items.iter().any(|item| item.id == *candidate))
        .unwrap_or_else(|| format!("step_{}", work_form.items.len() + 1))
}

fn apply_form_updates(work_form: &mut WorkForm, update: UpdateWorkFormInput) -> anyhow::Result<()> {
    anyhow::ensure!(
        !update.is_empty(),
        "form requires at least one changed field"
    );
    if let Some(objective) = update.objective {
        work_form.objective = validate_text("form.objective", objective, MAX_TEXT_CHARS)?;
    }
    if let Some(constraints) = update.constraints {
        work_form.constraints = validate_list("form.constraints", constraints)?;
    }
    if let Some(acceptance) = update.acceptance {
        work_form.acceptance = validate_list("form.acceptance", acceptance)?;
    }
    if let Some(status) = update.status {
        anyhow::ensure!(
            status != WorkFormStatus::Completed || work_form.blocking_items_complete(),
            "a completed WorkForm cannot contain unresolved blocking items"
        );
        work_form.set_status(status);
    }
    Ok(())
}

fn apply_item_updates(item: &mut WorkItem, updates: UpdateWorkItemInput) -> anyhow::Result<()> {
    if let Some(step) = updates.step {
        item.title = validate_text("updates.step", step, MAX_TEXT_CHARS)?;
    }
    if let Some(status) = updates.status {
        item.status = status;
        if !status.requires_note() {
            item.note = None;
        }
    }
    if let Some(disposition) = updates.completion_disposition {
        item.completion_disposition = disposition;
    }
    if let Some(depends_on) = updates.depends_on {
        item.depends_on = validate_ids("updates.depends_on", depends_on)?;
    }
    if let Some(note) = updates.note {
        item.note = Some(validate_text("updates.note", note, MAX_NOTE_CHARS)?);
    }
    if let Some(acceptance) = updates.acceptance {
        item.acceptance = validate_list("updates.acceptance", acceptance)?;
    }
    if let Some(evidence_refs) = updates.evidence_refs {
        item.evidence_refs = validate_list("updates.evidence_refs", evidence_refs)?;
    }
    Ok(())
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

fn work_form_result(
    call_id: Uuid,
    tool_name: &str,
    form: WorkForm,
    changed_item_id: Option<String>,
) -> anyhow::Result<ToolResult> {
    let next = form.next_runnable_item().cloned();
    let next_index = next
        .as_ref()
        .and_then(|next| form.items.iter().position(|item| item.id == next.id))
        .map(|index| index + 1);
    let value = serde_json::to_value(&form)?;
    Ok(ToolResult {
        call_id,
        output: form.render_for_model(),
        content: vec![ModelContentPart::json(value.clone())],
        metadata: json!({
            "toolName": tool_name,
            "workForm": value,
            "formId": form.id,
            "revision": form.revision,
            "status": form.status,
            "completed": form.completed_items(),
            "resolved": form.resolved_items(),
            "total": form.items.len(),
            "nextRunnableItem": next,
            "currentItemIndex": next_index,
            "changedItemId": changed_item_id,
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

    #[tokio::test]
    async fn set_and_update_mutate_the_same_native_work_form() {
        let thread_id = Uuid::new_v4();
        let turn_id = Uuid::new_v4();
        let created = SetPlanTool
            .execute(
                ToolCall::new(
                    "set_plan",
                    json!({
                        "objective": "Implement and verify",
                        "explanation": "Commit the known work",
                        "items": [{
                            "id": "implement",
                            "step": "Implement"
                        }]
                    }),
                ),
                context(thread_id, turn_id),
            )
            .await
            .expect("create WorkForm");
        let form: WorkForm =
            serde_json::from_value(created.metadata["workForm"].clone()).expect("WorkForm");
        assert_eq!(form.scope, WorkScope::Turn(turn_id));
        assert_eq!(form.revision, 1);

        let mut update_context = context(thread_id, turn_id);
        update_context.current_work_form = Some(form);
        let updated = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "explanation": "Implementation verified",
                        "items": [{
                            "id": "implement",
                            "status": "completed"
                        }]
                    }),
                ),
                update_context,
            )
            .await
            .expect("update WorkForm");
        let form: WorkForm =
            serde_json::from_value(updated.metadata["workForm"].clone()).expect("WorkForm");
        assert_eq!(form.revision, 2);
        assert_eq!(form.status, WorkFormStatus::Completed);
        assert_eq!(form.items[0].status, WorkItemStatus::Completed);
    }

    #[test]
    fn removed_plan_protocol_is_rejected_at_the_schema_boundary() {
        assert!(SetPlanTool
            .input_error(&json!({
                "expected_revision": 0,
                "objective": "work",
                "items": [{ "title": "Inspect" }]
            }))
            .is_some());
        assert!(UpdatePlanTool
            .input_error(&json!({
                "operation": "update_item",
                "item_id": "inspect",
                "updates": { "status": "completed" }
            }))
            .is_some());
    }

    #[test]
    fn execution_checklist_tools_reject_plan_mode() {
        let mut plan_context = context(Uuid::new_v4(), Uuid::new_v4());
        plan_context.collaboration_mode = CollaborationMode::Plan;

        let set_error = <SetPlanTool as TypedTool>::validate_context(&SetPlanTool, &plan_context)
            .expect_err("set_plan must reject Plan mode");
        assert!(set_error.to_string().contains("execution checklist tool"));

        let update_error =
            <UpdatePlanTool as TypedTool>::validate_context(&UpdatePlanTool, &plan_context)
                .expect_err("update_plan must reject Plan mode");
        assert!(update_error
            .to_string()
            .contains("execution checklist tool"));
    }

    #[tokio::test]
    async fn compact_plan_shape_generates_ids_and_batches_updates() {
        let thread_id = Uuid::new_v4();
        let turn_id = Uuid::new_v4();
        let created = SetPlanTool
            .execute(
                ToolCall::new(
                    "set_plan",
                    json!({
                        "objective": "Implement and verify",
                        "items": [
                            { "step": "Implement", "status": "in_progress" },
                            { "step": "Verify", "depends_on": ["step_1"] }
                        ]
                    }),
                ),
                context(thread_id, turn_id),
            )
            .await
            .expect("create compact WorkForm");
        let form: WorkForm =
            serde_json::from_value(created.metadata["workForm"].clone()).expect("WorkForm");
        assert_eq!(form.items[0].id, "step_1");
        assert_eq!(form.items[0].status, WorkItemStatus::InProgress);
        assert_eq!(form.items[1].id, "step_2");

        let mut update_context = context(thread_id, turn_id);
        update_context.current_work_form = Some(form);
        let updated = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "items": [
                            {
                                "id": "step_1",
                                "status": "completed"
                            },
                            { "id": "step_2", "status": "in_progress" }
                        ]
                    }),
                ),
                update_context,
            )
            .await
            .expect("batch compact WorkForm update");
        let form: WorkForm =
            serde_json::from_value(updated.metadata["workForm"].clone()).expect("WorkForm");
        assert_eq!(form.revision, 2);
        assert_eq!(form.items[0].status, WorkItemStatus::Completed);
        assert_eq!(form.items[1].status, WorkItemStatus::InProgress);

        let set_schema = SetPlanTool.schema().to_string();
        assert!(!set_schema.contains("expected_revision"));
        assert!(!set_schema.contains("change_reason"));
        assert!(set_schema.contains("\"step\""));
        assert!(!set_schema.contains("\"title\""));
        assert!(SetPlanTool
            .input_error(&json!({
                "objective": "Current state",
                "items": [{ "step": "Already done", "status": "completed" }]
            }))
            .is_none());
        let update_schema = UpdatePlanTool.schema().to_string();
        assert!(!update_schema.contains("expected_revision"));
        assert!(!update_schema.contains("item_id"));
        assert!(!update_schema.contains("\"operation\""));
        assert!(!update_schema.contains("change_reason"));
    }
}
