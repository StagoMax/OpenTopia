//! One durable work-state model shared by ordinary complex turns and Goals.

use crate::completion_runtime::{CompletionDisposition, CompletionSignal};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

const WORK_FORM_NAMESPACE: Uuid = Uuid::from_u128(0x6f70656e_746f_7069_615f_776f726b66);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum WorkScope {
    Turn(Uuid),
    Goal(Uuid),
}

impl WorkScope {
    pub fn id(self) -> Uuid {
        match self {
            Self::Turn(id) | Self::Goal(id) => id,
        }
    }

    pub fn kind(self) -> &'static str {
        match self {
            Self::Turn(_) => "turn",
            Self::Goal(_) => "goal",
        }
    }

    pub fn form_id(self) -> Uuid {
        Uuid::new_v5(
            &WORK_FORM_NAMESPACE,
            format!("{}:{}", self.kind(), self.id()).as_bytes(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkFormStatus {
    Active,
    Completed,
    Blocked,
    Paused,
    Cancelled,
}

impl WorkFormStatus {
    pub fn permits_invocation_end(self) -> bool {
        self != Self::Active
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
            Self::Paused => "paused",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "completed" => Ok(Self::Completed),
            "blocked" => Ok(Self::Blocked),
            "paused" => Ok(Self::Paused),
            "cancelled" => Ok(Self::Cancelled),
            other => anyhow::bail!("unknown work form status: {other}"),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemStatus {
    Pending,
    InProgress,
    Completed,
    Deferred,
    Blocked,
    Cancelled,
}

impl WorkItemStatus {
    pub fn is_actionable(self) -> bool {
        matches!(self, Self::Pending | Self::InProgress)
    }

    pub fn is_resolved(self) -> bool {
        self == Self::Completed
    }

    pub fn requires_note(self) -> bool {
        matches!(self, Self::Deferred | Self::Blocked | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkItem {
    pub id: String,
    pub title: String,
    pub status: WorkItemStatus,
    #[serde(default)]
    pub completion_disposition: CompletionDisposition,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkForm {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub scope: WorkScope,
    pub objective: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub acceptance: Vec<String>,
    pub status: WorkFormStatus,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_reason: Option<String>,
    #[serde(default)]
    pub items: Vec<WorkItem>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkForm {
    pub fn new(
        thread_id: Uuid,
        scope: WorkScope,
        objective: impl Into<String>,
        items: Vec<WorkItem>,
    ) -> Self {
        let now = Utc::now();
        let mut form = Self {
            id: scope.form_id(),
            thread_id,
            scope,
            objective: objective.into(),
            constraints: Vec::new(),
            acceptance: Vec::new(),
            status: WorkFormStatus::Active,
            revision: 0,
            change_reason: None,
            items,
            created_at: now,
            updated_at: now,
        };
        form.recalculate_status();
        form
    }

    pub fn empty_goal(thread_id: Uuid, goal_id: Uuid, objective: String) -> Self {
        Self::new(thread_id, WorkScope::Goal(goal_id), objective, Vec::new())
    }

    pub fn completed_items(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == WorkItemStatus::Completed)
            .count()
    }

    pub fn resolved_items(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status.is_resolved())
            .count()
    }

    pub fn next_runnable_item(&self) -> Option<&WorkItem> {
        self.items
            .iter()
            .find(|item| item.status == WorkItemStatus::InProgress)
            .or_else(|| {
                self.items.iter().find(|item| {
                    item.status == WorkItemStatus::Pending
                        && item.depends_on.iter().all(|dependency| {
                            self.items.iter().any(|candidate| {
                                candidate.id == *dependency
                                    && candidate.status == WorkItemStatus::Completed
                            })
                        })
                })
            })
    }

    pub fn recalculate_status(&mut self) {
        let blocking = self
            .items
            .iter()
            .filter(|item| item.completion_disposition == CompletionDisposition::Blocking)
            .collect::<Vec<_>>();
        self.status = if self.items.is_empty() {
            WorkFormStatus::Active
        } else if blocking.iter().any(|item| {
            matches!(
                item.status,
                WorkItemStatus::Blocked | WorkItemStatus::Cancelled
            )
        }) {
            WorkFormStatus::Blocked
        } else if blocking
            .iter()
            .any(|item| item.status == WorkItemStatus::Deferred)
        {
            WorkFormStatus::Paused
        } else if blocking.iter().any(|item| item.status.is_actionable()) {
            WorkFormStatus::Active
        } else {
            // Advisory work may remain visible and continue to emit reminders;
            // it never prevents the blocking contract from completing.
            WorkFormStatus::Completed
        };
        self.updated_at = Utc::now();
    }

    pub fn blocking_items_complete(&self) -> bool {
        !self.items.is_empty()
            && self.items.iter().all(|item| {
                item.completion_disposition == CompletionDisposition::Advisory
                    || item.status == WorkItemStatus::Completed
            })
    }

    pub fn set_status(&mut self, status: WorkFormStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.objective.trim().is_empty(),
            "work form objective cannot be empty"
        );
        anyhow::ensure!(
            self.status != WorkFormStatus::Completed || self.blocking_items_complete(),
            "completed work form still has unresolved blocking items"
        );
        let mut ids = HashSet::new();
        let mut titles = HashSet::new();
        let mut in_progress = 0usize;
        for item in &self.items {
            anyhow::ensure!(!item.id.trim().is_empty(), "work item id cannot be empty");
            anyhow::ensure!(
                !item.title.trim().is_empty(),
                "work item title cannot be empty"
            );
            anyhow::ensure!(
                ids.insert(item.id.as_str()),
                "duplicate work item id: {}",
                item.id
            );
            anyhow::ensure!(
                titles.insert(item.title.to_lowercase()),
                "duplicate work item title: {}",
                item.title
            );
            in_progress += usize::from(item.status == WorkItemStatus::InProgress);
            anyhow::ensure!(
                !item.status.requires_note()
                    || item
                        .note
                        .as_deref()
                        .is_some_and(|note| !note.trim().is_empty()),
                "work item {} requires a note for {:?}",
                item.id,
                item.status
            );
            for dependency in &item.depends_on {
                anyhow::ensure!(
                    dependency != &item.id,
                    "work item {} cannot depend on itself",
                    item.id
                );
                let dependency_item = self
                    .items
                    .iter()
                    .find(|candidate| &candidate.id == dependency)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "work item {} has unknown dependency: {dependency}",
                            item.id
                        )
                    })?;
                if matches!(
                    item.status,
                    WorkItemStatus::InProgress | WorkItemStatus::Completed
                ) {
                    anyhow::ensure!(
                        dependency_item.status == WorkItemStatus::Completed,
                        "work item {} cannot be {:?} before dependency {dependency} is completed",
                        item.id,
                        item.status
                    );
                }
            }
            if item.status == WorkItemStatus::Completed {
                anyhow::ensure!(
                    !item.acceptance.is_empty(),
                    "completed work item {} requires acceptance criteria",
                    item.id
                );
                anyhow::ensure!(
                    !item.evidence_refs.is_empty(),
                    "completed work item {} requires evidence refs",
                    item.id
                );
            }
        }
        anyhow::ensure!(
            in_progress <= 1,
            "work form may contain at most one in_progress item"
        );

        let mut unresolved = self
            .items
            .iter()
            .map(|item| {
                (
                    item.id.clone(),
                    item.depends_on.iter().cloned().collect::<HashSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        while !unresolved.is_empty() {
            let resolved = unresolved
                .iter()
                .filter_map(|(id, dependencies)| dependencies.is_empty().then_some(id.clone()))
                .collect::<Vec<_>>();
            anyhow::ensure!(
                !resolved.is_empty(),
                "work form contains a dependency cycle involving: {}",
                unresolved.keys().cloned().collect::<Vec<_>>().join(", ")
            );
            for id in &resolved {
                unresolved.remove(id);
            }
            for dependencies in unresolved.values_mut() {
                for id in &resolved {
                    dependencies.remove(id);
                }
            }
        }
        Ok(())
    }

    pub fn render_for_model(&self) -> String {
        let mut lines = vec![
            format!("Objective: {}", self.objective),
            format!("Work form revision: {}", self.revision),
            format!("Status: {:?}", self.status),
        ];
        if let Some(reason) = self.change_reason.as_deref() {
            lines.push(format!("Last change: {reason}"));
        }
        for item in &self.items {
            lines.push(format!("- {} [{:?}]: {}", item.id, item.status, item.title));
            if let Some(note) = item.note.as_deref() {
                lines.push(format!("  Note: {note}"));
            }
            if !item.depends_on.is_empty() {
                lines.push(format!("  Depends on: {}", item.depends_on.join(", ")));
            }
            for acceptance in &item.acceptance {
                lines.push(format!("  Acceptance: {acceptance}"));
            }
            for evidence in &item.evidence_refs {
                lines.push(format!("  Evidence: {evidence}"));
            }
        }
        lines.join("\n")
    }

    pub fn completion_signals(&self) -> Vec<CompletionSignal> {
        if matches!(
            self.status,
            WorkFormStatus::Blocked | WorkFormStatus::Paused | WorkFormStatus::Cancelled
        ) {
            return Vec::new();
        }
        if self.items.is_empty() {
            return matches!(self.scope, WorkScope::Goal(_))
                .then(|| {
                    CompletionSignal::blocking(
                        format!("work_form:{}", self.id),
                        "The Goal work form has no committed work items yet.",
                        serde_json::json!({
                            "kind": "work_form_empty",
                            "formId": self.id,
                            "scope": self.scope,
                            "revision": self.revision,
                        }),
                    )
                })
                .into_iter()
                .collect();
        }
        self.items
            .iter()
            .filter(|item| item.status.is_actionable())
            .map(|item| CompletionSignal {
                source_id: format!("work_form:{}:{}", self.id, item.id),
                disposition: item.completion_disposition,
                message: format!("Work item `{}` is still {:?}.", item.title, item.status),
                details: serde_json::json!({
                    "kind": "work_item_pending",
                    "formId": self.id,
                    "scope": self.scope,
                    "revision": self.revision,
                    "itemId": item.id,
                    "title": item.title,
                    "status": item.status,
                }),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_and_goal_forms_never_share_an_identity() {
        let id = Uuid::new_v4();
        assert_ne!(WorkScope::Turn(id).form_id(), WorkScope::Goal(id).form_id());
        assert_eq!(WorkScope::Goal(id).form_id(), WorkScope::Goal(id).form_id());
    }

    #[test]
    fn only_active_actionable_items_emit_completion_signals() {
        let mut form = WorkForm::empty_goal(Uuid::new_v4(), Uuid::new_v4(), "ship".into());
        assert_eq!(form.completion_signals().len(), 1);
        form.items.push(WorkItem {
            id: "implement".into(),
            title: "Implement".into(),
            status: WorkItemStatus::Pending,
            completion_disposition: CompletionDisposition::Blocking,
            depends_on: Vec::new(),
            note: None,
            acceptance: Vec::new(),
            evidence_refs: Vec::new(),
        });
        assert_eq!(form.completion_signals().len(), 1);
        form.status = WorkFormStatus::Blocked;
        assert!(form.completion_signals().is_empty());
    }

    #[test]
    fn only_completed_blocking_items_satisfy_form_completion() {
        let mut form = WorkForm::new(
            Uuid::new_v4(),
            WorkScope::Turn(Uuid::new_v4()),
            "ship",
            vec![
                WorkItem {
                    id: "blocking".into(),
                    title: "Blocking".into(),
                    status: WorkItemStatus::Cancelled,
                    completion_disposition: CompletionDisposition::Blocking,
                    depends_on: Vec::new(),
                    note: Some("cannot proceed".into()),
                    acceptance: vec!["verified".into()],
                    evidence_refs: Vec::new(),
                },
                WorkItem {
                    id: "advisory".into(),
                    title: "Advisory".into(),
                    status: WorkItemStatus::Pending,
                    completion_disposition: CompletionDisposition::Advisory,
                    depends_on: Vec::new(),
                    note: None,
                    acceptance: Vec::new(),
                    evidence_refs: Vec::new(),
                },
            ],
        );
        form.recalculate_status();
        assert_eq!(form.status, WorkFormStatus::Blocked);
        assert!(!form.blocking_items_complete());

        let blocking = form
            .items
            .iter_mut()
            .find(|item| item.id == "blocking")
            .expect("blocking item");
        blocking.status = WorkItemStatus::Completed;
        blocking.note = None;
        blocking.evidence_refs = vec!["test:passed".into()];
        form.recalculate_status();
        assert_eq!(form.status, WorkFormStatus::Completed);
        assert!(form.blocking_items_complete());
        let signals = form.completion_signals();
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].disposition, CompletionDisposition::Advisory);
    }
}
