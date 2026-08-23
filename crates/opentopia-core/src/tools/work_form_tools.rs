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
    /// Legacy optimistic-concurrency field. New calls use the active revision.
    #[serde(default)]
    expected_revision: Option<u64>,
    /// Required for a new ordinary Turn form. Goal forms inherit the server objective.
    #[serde(default)]
    objective: Option<String>,
    /// Optional concise explanation for replacing the plan.
    #[serde(default)]
    explanation: Option<String>,
    /// Legacy explanation field retained for static-schema compatibility.
    #[serde(default)]
    change_reason: Option<String>,
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
    /// Optional stable ID. Omit to generate step_1, step_2, and so on.
    #[serde(default)]
    id: Option<String>,
    /// Concise executable step.
    #[serde(default)]
    step: Option<String>,
    /// Legacy step field retained for static-schema compatibility.
    #[serde(default)]
    title: Option<String>,
    /// Initial execution state. Creation may start at most one item; terminal
    /// states still require update_plan with their ordinary evidence/reason rules.
    #[serde(default)]
    status: Option<InitialWorkItemStatus>,
    #[serde(default)]
    completion_disposition: CompletionDisposition,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    #[schemars(length(max = 20))]
    acceptance: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum InitialWorkItemStatus {
    Pending,
    InProgress,
}

impl From<InitialWorkItemStatus> for WorkItemStatus {
    fn from(status: InitialWorkItemStatus) -> Self {
        match status {
            InitialWorkItemStatus::Pending => Self::Pending,
            InitialWorkItemStatus::InProgress => Self::InProgress,
        }
    }
}

pub struct SetPlanTool;

#[async_trait]
impl TypedTool for SetPlanTool {
    type Input = SetWorkFormInput;

    fn name(&self) -> &str {
        "set_plan"
    }

    fn description(&self) -> &str {
        "Create or replace the current task plan as external memory. Use only when the planning policy warrants it; plans are optional for simple or focused work. Provide an objective and concise executable items with a step; item IDs are optional and otherwise generated as step_1, step_2, and so on. Dependencies and acceptance criteria remain optional. Items start pending unless status is in_progress; at most one item may start in progress. Use update_plan for completed, blocked, deferred, or cancelled states."
    }

    fn validate_context(&self, ctx: &ToolInvocationContext) -> anyhow::Result<()> {
        anyhow::ensure!(
            ctx.agent_depth == 0,
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
        if let Some(expected_revision) = input.expected_revision {
            anyhow::ensure!(
                observed_revision == expected_revision,
                "stale WorkForm revision: expected {expected_revision}, current {observed_revision}"
            );
        }
        let objective = match input.objective {
            Some(objective) => validate_text("objective", objective, MAX_TEXT_CHARS)?,
            None => existing
                .map(|form| form.objective.clone())
                .context("a new Turn WorkForm requires objective")?,
        };
        let mut items = Vec::with_capacity(input.items.len());
        for (index, item) in input.items.into_iter().enumerate() {
            let id = item.id.unwrap_or_else(|| format!("step_{}", index + 1));
            let step = compact_or_legacy("item step", item.step, item.title)?
                .context("item requires step")?;
            let status = item
                .status
                .map(WorkItemStatus::from)
                .unwrap_or(WorkItemStatus::Pending);
            items.push(WorkItem {
                id: validate_text("item.id", id, MAX_ID_CHARS)?,
                title: validate_text("item.step", step, MAX_TEXT_CHARS)?,
                status,
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
        form.change_reason =
            compact_or_legacy("plan explanation", input.explanation, input.change_reason)?
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
    /// Optional stable ID. Omit to generate the next step_N ID.
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    step: Option<String>,
    /// Legacy step field retained for static-schema compatibility.
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    status: Option<WorkItemStatus>,
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
    step: Option<String>,
    /// Legacy step field retained for static-schema compatibility.
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
        self.step.is_none()
            && self.title.is_none()
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
    append: Vec<AppendWorkItemInput>,
    /// Remove item IDs after applying updates.
    #[serde(default)]
    remove: Vec<String>,
    /// Optional concise explanation for this update.
    #[serde(default)]
    explanation: Option<String>,
    /// Legacy explanation field retained for static-schema compatibility.
    #[serde(default)]
    change_reason: Option<String>,
    /// Legacy single-operation shape.
    #[serde(default)]
    operation: Option<WorkFormOperation>,
    #[serde(default)]
    expected_revision: Option<u64>,
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
        "Update the current advisory task plan atomically after a plan has been created. The compact shape batches existing item patches in items, new steps in append, removals in remove, and optional form changes; the active revision is supplied by the runtime. Completed items require acceptance criteria and evidence refs. Blocked, deferred, and cancelled items require a note."
    }

    fn validate_context(&self, ctx: &ToolInvocationContext) -> anyhow::Result<()> {
        anyhow::ensure!(
            ctx.agent_depth == 0,
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
        if let Some(expected_revision) = input.expected_revision {
            anyhow::ensure!(
                work_form.revision == expected_revision,
                "WorkForm revision conflict: expected {expected_revision}, current {}",
                work_form.revision
            );
        }
        let explicit_form_status = input.form.as_ref().and_then(|form| form.status);
        let legacy_operation = input.operation;

        let changed_item_id = if let Some(operation) = legacy_operation {
            anyhow::ensure!(
                input.items.is_empty() && input.append.is_empty() && input.remove.is_empty(),
                "legacy update_plan operation cannot be combined with compact items/append/remove"
            );
            match operation {
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
                    let generated_id = format!("step_{}", work_form.items.len() + 1);
                    let item = validate_item(item, generated_id)?;
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
                            status != WorkFormStatus::Completed
                                || work_form.blocking_items_complete(),
                            "a completed WorkForm cannot contain unresolved blocking items"
                        );
                        work_form.set_status(status);
                    }
                    None
                }
            }
        } else {
            anyhow::ensure!(
                input.item_id.is_none() && input.item.is_none() && input.updates.is_none(),
                "compact update_plan does not accept legacy item_id/item/updates"
            );
            apply_compact_plan_update(
                &mut work_form,
                input.items,
                input.append,
                input.remove,
                input.form,
            )?
        };

        if legacy_operation != Some(WorkFormOperation::UpdateForm) || explicit_form_status.is_none()
        {
            work_form.recalculate_status();
        }
        work_form.revision = work_form
            .revision
            .checked_add(1)
            .context("WorkForm revision overflow")?;
        work_form.change_reason =
            compact_or_legacy("plan explanation", input.explanation, input.change_reason)?
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

fn validate_item(input: AppendWorkItemInput, generated_id: String) -> anyhow::Result<WorkItem> {
    let step =
        compact_or_legacy("item step", input.step, input.title)?.context("item requires step")?;
    Ok(WorkItem {
        id: validate_text("item.id", input.id.unwrap_or(generated_id), MAX_ID_CHARS)?,
        title: validate_text("item.step", step, MAX_TEXT_CHARS)?,
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

fn apply_compact_plan_update(
    work_form: &mut WorkForm,
    items: Vec<CompactWorkItemUpdateInput>,
    append: Vec<AppendWorkItemInput>,
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
    if let Some(step) = compact_or_legacy("updates step", updates.step, updates.title)? {
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

fn compact_or_legacy(
    field: &str,
    compact: Option<String>,
    legacy: Option<String>,
) -> anyhow::Result<Option<String>> {
    anyhow::ensure!(
        compact.is_none() || legacy.is_none(),
        "{field} accepts either the compact field or its legacy alias, not both"
    );
    Ok(compact.or(legacy))
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
                                "status": "completed",
                                "acceptance": ["Implementation complete"],
                                "evidence_refs": ["focused test passed"]
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
        assert!(set_schema.contains("expected_revision"));
        assert!(set_schema.contains("change_reason"));
        assert!(set_schema.contains("\"step\""));
        assert!(set_schema.contains("\"title\""));
        assert!(SetPlanTool
            .input_error(&json!({
                "objective": "Invalid initial state",
                "items": [{ "step": "Already done", "status": "completed" }]
            }))
            .is_some());
        let update_schema = UpdatePlanTool.schema().to_string();
        assert!(update_schema.contains("expected_revision"));
        assert!(update_schema.contains("item_id"));
        assert!(update_schema.contains("\"operation\""));
        assert!(update_schema.contains("change_reason"));
    }
}
