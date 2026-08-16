//! Product-neutral completion signals and the final-candidate gate.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompletionDisposition {
    /// The current final candidate must be rejected and returned to the model.
    Blocking,
    /// The turn may finish; the signal is only surfaced as a reminder.
    Advisory,
}

impl Default for CompletionDisposition {
    fn default() -> Self {
        Self::Blocking
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionSignal {
    pub source_id: String,
    pub disposition: CompletionDisposition,
    pub message: String,
    #[serde(default)]
    pub details: Value,
}

impl CompletionSignal {
    pub fn blocking(
        source_id: impl Into<String>,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            disposition: CompletionDisposition::Blocking,
            message: message.into(),
            details,
        }
    }

    pub fn advisory(
        source_id: impl Into<String>,
        message: impl Into<String>,
        details: Value,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            disposition: CompletionDisposition::Advisory,
            message: message.into(),
            details,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionReport {
    pub blockers: Vec<CompletionSignal>,
    pub reminders: Vec<CompletionSignal>,
}

/// Classifies completion signals without understanding their domain payload.
pub trait CompletionGate: Send + Sync {
    fn check(&self, signals: Vec<CompletionSignal>) -> CompletionReport;
}

/// Resolves registered completion forms into product-neutral signals. The
/// registry understands WorkForm state, while the gate below only classifies
/// dispositions and therefore contains no Plan or Goal business rules.
pub trait CompletionRegistry: Send + Sync {
    fn signals(&self, forms: &[crate::work_form::WorkForm]) -> Vec<CompletionSignal>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultCompletionRegistry;

impl CompletionRegistry for DefaultCompletionRegistry {
    fn signals(&self, forms: &[crate::work_form::WorkForm]) -> Vec<CompletionSignal> {
        forms
            .iter()
            .flat_map(crate::work_form::WorkForm::completion_signals)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultCompletionGate;

impl CompletionGate for DefaultCompletionGate {
    fn check(&self, signals: Vec<CompletionSignal>) -> CompletionReport {
        let mut report = CompletionReport::default();
        for signal in signals {
            match signal.disposition {
                CompletionDisposition::Blocking => report.blockers.push(signal),
                CompletionDisposition::Advisory => report.reminders.push(signal),
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn advisories_never_become_completion_blockers() {
        let gate: &dyn CompletionGate = &DefaultCompletionGate;
        let report = gate.check(vec![
            CompletionSignal::advisory(
                "background:build",
                "build is still running",
                json!({ "jobId": "build" }),
            ),
            CompletionSignal::blocking(
                "turn-form:step-1",
                "required step is pending",
                json!({ "stepId": "step-1" }),
            ),
        ]);

        assert_eq!(report.blockers.len(), 1);
        assert_eq!(report.reminders.len(), 1);
        assert_eq!(report.blockers[0].source_id, "turn-form:step-1");
        assert_eq!(report.reminders[0].source_id, "background:build");
    }

    #[test]
    fn registry_reads_work_forms_and_gate_only_classifies_dispositions() {
        use crate::work_form::{WorkForm, WorkItem, WorkItemStatus, WorkScope};

        let thread_id = Uuid::new_v4();
        let turn_id = Uuid::new_v4();
        let form = WorkForm::new(
            thread_id,
            WorkScope::Turn(turn_id),
            "complex-turn",
            vec![
                WorkItem {
                    id: "required".into(),
                    title: "Required".into(),
                    status: WorkItemStatus::Pending,
                    completion_disposition: CompletionDisposition::Blocking,
                    note: None,
                    depends_on: Vec::new(),
                    acceptance: Vec::new(),
                    evidence_refs: Vec::new(),
                },
                WorkItem {
                    id: "background".into(),
                    title: "Background".into(),
                    status: WorkItemStatus::InProgress,
                    completion_disposition: CompletionDisposition::Advisory,
                    note: None,
                    depends_on: Vec::new(),
                    acceptance: Vec::new(),
                    evidence_refs: Vec::new(),
                },
            ],
        );
        let signals = DefaultCompletionRegistry.signals(&[form]);
        let report = DefaultCompletionGate.check(signals);
        assert_eq!(report.blockers.len(), 1);
        assert_eq!(report.reminders.len(), 1);
    }
}
