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
    /// Revision currently observed by the caller. Use 0 when no form exists.
    expected_revision: u64,
    /// Required for a new ordinary Turn form. Goal forms inherit the server objective.
    #[serde(default)]
    objective: Option<String>,
    /// Why the committed work set is being created or replaced.
    change_reason: String,
    #[serde(default)]
    constraints: Vec<String>,
    #[serde(default)]
    acceptance: Vec<String>,
    #[schemars(length(min = 1, max = 20))]
    items: Vec<SetWorkItemInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetWorkItemInput {
    id: String,
    title: String,
    #[serde(default)]
    completion_disposition: CompletionDisposition,
    #[serde(default)]
    depends_on: Vec<String>,
    #[schemars(length(min = 1, max = 20))]
    acceptance: Vec<String>,
}

pub struct SetPlanTool;

#[async_trait]
impl TypedTool for SetPlanTool {
    type Input = SetWorkFormInput;

    fn name(&self) -> &str {
        "set_plan"
    }

    fn description(&self) -> &str {
        "Create or atomically replace the current WorkForm used as external memory for a genuinely complex task. Every item starts pending, dependencies are explicit, and completion_disposition distinguishes blocking work from advisory background work. This tool records commitments and progress; it never creates a separate Plan execution mode."
    }

    fn validate_context(&self, ctx: &ToolInvocationContext) -> anyhow::Result<()> {
        anyhow::ensure!(
            ctx.subagent_depth == 0,
            "only the parent agent may set the shared WorkForm"
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
        anyhow::ensure!(
            observed_revision == input.expected_revision,
            "stale WorkForm revision: expected {}, current {}",
            input.expected_revision,
            observed_revision
        );
        let objective = match input.objective {
            Some(objective) => validate_text("objective", objective, MAX_TEXT_CHARS)?,
            None => existing
                .map(|form| form.objective.clone())
                .context("a new Turn WorkForm requires objective")?,
        };
        let mut items = Vec::with_capacity(input.items.len());
        for item in input.items {
            items.push(WorkItem {
                id: validate_text("item.id", item.id, MAX_ID_CHARS)?,
                title: validate_text("item.title", item.title, MAX_TEXT_CHARS)?,
                status: WorkItemStatus::Pending,
                completion_disposition: item.completion_disposition,
                depends_on: validate_ids("item.depends_on", item.depends_on)?,
                note: None,
                acceptance: validate_list("item.acceptance", item.acceptance)?,
                evidence_refs: Vec::new(),
            });
        }
        let mut form = WorkForm::new(thread_id, scope, objective, items);
        form.constraints = validate_list("constraints", input.constraints)?;
        form.acceptance = validate_list("acceptance", input.acceptance)?;
        form.change_reason = Some(validate_text(
            "change_reason",
            input.change_reason,
            MAX_REASON_CHARS,
        )?);
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum WorkFormOperation {
    AppendItem,
    UpdateItem,
    RemoveItem,
    UpdateForm,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AppendWorkItemInput {
    id: String,
    title: String,
    status: WorkItemStatus,
    #[serde(default)]
    completion_disposition: CompletionDisposition,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    acceptance: Vec<String>,
    #[serde(default)]
    evidence_refs: Vec<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateWorkItemInput {
    #[serde(default)]
    title: Option<String>,
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
        self.title.is_none()
            && self.status.is_none()
            && self.completion_disposition.is_none()
            && self.depends_on.is_none()
            && self.note.is_none()
            && self.acceptance.is_none()
            && self.evidence_refs.is_none()
    }
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
    operation: WorkFormOperation,
    expected_revision: u64,
    change_reason: String,
    #[serde(default)]
    item_id: Option<String>,
    #[serde(default)]
    item: Option<AppendWorkItemInput>,
    #[serde(default)]
    updates: Option<UpdateWorkItemInput>,
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
        "Apply one atomic append_item, update_item, remove_item, or update_form mutation to the current WorkForm. Send expected_revision for optimistic concurrency. Completed items require acceptance criteria and evidence_refs. Blocked, deferred, and cancelled items require a note. The returned runnable item is advisory, not a scheduler gate."
    }

    fn validate_context(&self, ctx: &ToolInvocationContext) -> anyhow::Result<()> {
        anyhow::ensure!(
            ctx.subagent_depth == 0,
            "only the parent agent may update the shared WorkForm"
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
        anyhow::ensure!(
            work_form.revision == input.expected_revision,
            "WorkForm revision conflict: expected {}, current {}",
            input.expected_revision,
            work_form.revision
        );
        let explicit_form_status = input.form.as_ref().and_then(|form| form.status);

        let changed_item_id = match input.operation {
            WorkFormOperation::AppendItem => {
                anyhow::ensure!(
                    input.item_id.is_none() && input.updates.is_none() && input.form.is_none(),
                    "append_item accepts only item"
                );
                let item = input.item.context("append_item requires item")?;
                anyhow::ensure!(
                    work_form.items.len() < MAX_ITEMS,
                    "WorkForm may contain at most {MAX_ITEMS} items"
                );
                let item = validate_item(item)?;
                anyhow::ensure!(
                    !work_form.items.iter().any(|current| current.id == item.id),
                    "WorkForm already contains item id: {}",
                    item.id
                );
                let id = item.id.clone();
                work_form.items.push(item);
                Some(id)
            }
            WorkFormOperation::UpdateItem => {
                anyhow::ensure!(
                    input.item.is_none() && input.form.is_none(),
                    "update_item accepts only item_id and updates"
                );
                let item_id = validate_text(
                    "item_id",
                    input.item_id.context("update_item requires item_id")?,
                    MAX_ID_CHARS,
                )?;
                let updates = input.updates.context("update_item requires updates")?;
                anyhow::ensure!(
                    !updates.is_empty(),
                    "update_item requires at least one changed field"
                );
                let item = work_form
                    .items
                    .iter_mut()
                    .find(|item| item.id == item_id)
                    .with_context(|| format!("WorkForm does not contain item id: {item_id}"))?;
                apply_item_updates(item, updates)?;
                Some(item_id)
            }
            WorkFormOperation::RemoveItem => {
                anyhow::ensure!(
                    input.item.is_none() && input.updates.is_none() && input.form.is_none(),
                    "remove_item accepts only item_id"
                );
                let item_id = validate_text(
                    "item_id",
                    input.item_id.context("remove_item requires item_id")?,
                    MAX_ID_CHARS,
                )?;
                let dependents = work_form
                    .items
                    .iter()
                    .filter(|item| item.depends_on.contains(&item_id))
                    .map(|item| item.id.clone())
                    .collect::<Vec<_>>();
                anyhow::ensure!(
                    dependents.is_empty(),
                    "cannot remove item {item_id}; required by: {}",
                    dependents.join(", ")
                );
                let index = work_form
                    .items
                    .iter()
                    .position(|item| item.id == item_id)
                    .with_context(|| format!("WorkForm does not contain item id: {item_id}"))?;
                work_form.items.remove(index);
                None
            }
            WorkFormOperation::UpdateForm => {
                anyhow::ensure!(
                    input.item_id.is_none() && input.item.is_none() && input.updates.is_none(),
                    "update_form accepts only form"
                );
                let update = input.form.context("update_form requires form")?;
                anyhow::ensure!(
                    !update.is_empty(),
                    "update_form requires at least one changed field"
                );
                if let Some(objective) = update.objective {
                    work_form.objective =
                        validate_text("form.objective", objective, MAX_TEXT_CHARS)?;
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
                None
            }
        };

        if input.operation != WorkFormOperation::UpdateForm || explicit_form_status.is_none() {
            work_form.recalculate_status();
        }
        work_form.revision = work_form
            .revision
            .checked_add(1)
            .context("WorkForm revision overflow")?;
        work_form.change_reason = Some(validate_text(
            "change_reason",
            input.change_reason,
            MAX_REASON_CHARS,
        )?);
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
        .or_else(|| ctx.parent_turn_id.map(WorkScope::Turn))
        .context("WorkForm tools require a Goal or Turn scope")?;
    Ok((thread_id, scope))
}

fn validate_item(input: AppendWorkItemInput) -> anyhow::Result<WorkItem> {
    Ok(WorkItem {
        id: validate_text("item.id", input.id, MAX_ID_CHARS)?,
        title: validate_text("item.title", input.title, MAX_TEXT_CHARS)?,
        status: input.status,
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

fn apply_item_updates(item: &mut WorkItem, updates: UpdateWorkItemInput) -> anyhow::Result<()> {
    if let Some(title) = updates.title {
        item.title = validate_text("updates.title", title, MAX_TEXT_CHARS)?;
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
        context.parent_turn_id = Some(turn_id);
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
                        "expected_revision": 0,
                        "objective": "Implement and verify",
                        "change_reason": "Commit the known work",
                        "items": [{
                            "id": "implement",
                            "title": "Implement",
                            "depends_on": [],
                            "acceptance": ["Focused checks pass"]
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
                        "operation": "update_item",
                        "expected_revision": 1,
                        "change_reason": "Implementation verified",
                        "item_id": "implement",
                        "updates": {
                            "status": "completed",
                            "evidence_refs": ["cargo test -p opentopia-core passed"]
                        }
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

    #[tokio::test]
    async fn scope_is_runtime_owned_and_revision_conflicts_fail_closed() {
        let thread_id = Uuid::new_v4();
        let turn_id = Uuid::new_v4();
        let mut update_context = context(thread_id, turn_id);
        let mut form = WorkForm::new(thread_id, WorkScope::Turn(turn_id), "work", Vec::new());
        form.revision = 4;
        update_context.current_work_form = Some(form);

        let error = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "operation": "append_item",
                        "expected_revision": 3,
                        "change_reason": "stale",
                        "item": {
                            "id": "inspect",
                            "title": "Inspect",
                            "status": "pending"
                        }
                    }),
                ),
                update_context,
            )
            .await
            .expect_err("stale revision");
        assert!(error.to_string().contains("revision conflict"));
    }
}
