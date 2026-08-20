use crate::base_prompt::BASE_PROMPT_MODULES;
use crate::model_context::{estimate_tokens, ContextItemKind, ModelContextItem};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One node in the local attribution tree for a logical model request.
///
/// Providers report request-level usage totals. These nodes preserve the
/// harness-owned structure used to assemble that request without presenting
/// the individual values as provider-billed facts.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenEstimateDetail {
    pub id: String,
    pub label: String,
    pub tokens: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TokenEstimateDetail>,
}

impl TokenEstimateDetail {
    pub fn leaf(id: impl Into<String>, label: impl Into<String>, tokens: usize) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tokens,
            children: Vec::new(),
        }
    }

    pub fn branch(
        id: impl Into<String>,
        label: impl Into<String>,
        tokens: usize,
        children: Vec<TokenEstimateDetail>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tokens,
            children,
        }
    }
}

/// Provider-neutral estimate of the logical input carried by one model request.
///
/// These values are intentionally kept separate from provider-reported usage:
/// they explain which harness modules built the request, while provider usage is
/// the billing/accounting authority after the request completes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenEstimateBreakdown {
    pub base_instructions: usize,
    pub developer_instructions: usize,
    pub repository_instructions: usize,
    pub runtime_context: usize,
    pub skill_instructions: usize,
    pub summaries: usize,
    pub checkpoints: usize,
    pub conversation: usize,
    pub current_user: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
    /// Full input schemas sent directly in the request's tool surface.
    #[serde(default)]
    pub direct_tool_schemas: usize,
    /// Names/descriptions visible before a deferred tool is selected.
    #[serde(default)]
    pub deferred_tool_catalog: usize,
    /// Schemas appended by a provider Tool Search continuation.
    #[serde(default)]
    pub loaded_tool_schemas: usize,
    /// Sum of the three tool-surface buckets above. Counted in `total` once.
    pub tool_schemas: usize,
    /// A structured-output schema is a request field, not a tool definition.
    #[serde(default)]
    pub output_schema: usize,
    /// Provider-native assistant message items replayed inside the active turn.
    /// These are neither durable conversation history nor opaque continuation
    /// state, so they remain a separate, mutually exclusive bucket.
    #[serde(default)]
    pub turn_assistant_state: usize,
    pub provider_state: usize,
    pub other: usize,
    pub total: usize,
    /// Hierarchical local attribution. Omitted when replaying legacy logs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<TokenEstimateDetail>,
}

impl TokenEstimateBreakdown {
    pub fn from_context_items(items: &[ModelContextItem]) -> Self {
        let mut breakdown = Self::default();
        for item in items {
            breakdown.add_context_item(item.kind, item.token_estimate);
        }

        for (id, label, kinds) in CONTEXT_ROOTS {
            let children = merge_sibling_details(
                items
                    .iter()
                    .filter(|item| kinds.contains(&item.kind))
                    .flat_map(context_item_details)
                    .collect::<Vec<_>>(),
            );
            let tokens = children.iter().map(|child| child.tokens).sum();
            if tokens > 0 {
                breakdown.set_detail_root(*id, *label, tokens, children);
            }
        }
        breakdown.recalculate_total();
        breakdown
    }

    pub fn add_context_item(&mut self, kind: ContextItemKind, tokens: usize) {
        let bucket = match kind {
            ContextItemKind::BaseInstructions => &mut self.base_instructions,
            ContextItemKind::DeveloperInstructions => &mut self.developer_instructions,
            ContextItemKind::RepositoryInstructions => &mut self.repository_instructions,
            ContextItemKind::Environment
            | ContextItemKind::WorldState
            | ContextItemKind::CapabilityCatalog => &mut self.runtime_context,
            ContextItemKind::SkillInstructions | ContextItemKind::Skill => {
                &mut self.skill_instructions
            }
            ContextItemKind::Summary => &mut self.summaries,
            ContextItemKind::Checkpoint => &mut self.checkpoints,
            ContextItemKind::Conversation => &mut self.conversation,
            ContextItemKind::User => &mut self.current_user,
            ContextItemKind::ToolCall => &mut self.tool_calls,
            ContextItemKind::ToolResult => &mut self.tool_results,
        };
        *bucket = bucket.saturating_add(tokens);
    }

    pub fn set_detail_root(
        &mut self,
        id: impl Into<String>,
        label: impl Into<String>,
        tokens: usize,
        children: Vec<TokenEstimateDetail>,
    ) {
        let id = id.into();
        let detail = TokenEstimateDetail::branch(id.clone(), label, tokens, children);
        if let Some(existing) = self.details.iter_mut().find(|item| item.id == id) {
            *existing = detail;
        } else {
            self.details.push(detail);
        }
    }

    pub fn recalculate_total(&mut self) {
        self.total = self
            .base_instructions
            .saturating_add(self.developer_instructions)
            .saturating_add(self.repository_instructions)
            .saturating_add(self.runtime_context)
            .saturating_add(self.skill_instructions)
            .saturating_add(self.summaries)
            .saturating_add(self.checkpoints)
            .saturating_add(self.conversation)
            .saturating_add(self.current_user)
            .saturating_add(self.tool_calls)
            .saturating_add(self.tool_results)
            .saturating_add(self.tool_schemas)
            .saturating_add(self.output_schema)
            .saturating_add(self.turn_assistant_state)
            .saturating_add(self.provider_state)
            .saturating_add(self.other);
    }
}

type ContextRoot = (&'static str, &'static str, &'static [ContextItemKind]);

const CONTEXT_ROOTS: &[ContextRoot] = &[
    (
        "base_instructions",
        "Base instructions",
        &[ContextItemKind::BaseInstructions],
    ),
    (
        "developer_instructions",
        "Developer instructions",
        &[ContextItemKind::DeveloperInstructions],
    ),
    (
        "repository_instructions",
        "Repository instructions",
        &[ContextItemKind::RepositoryInstructions],
    ),
    (
        "runtime_context",
        "Runtime context",
        &[
            ContextItemKind::Environment,
            ContextItemKind::WorldState,
            ContextItemKind::CapabilityCatalog,
        ],
    ),
    (
        "skill_instructions",
        "Skill instructions",
        &[ContextItemKind::SkillInstructions, ContextItemKind::Skill],
    ),
    (
        "summaries",
        "Context summaries",
        &[ContextItemKind::Summary],
    ),
    ("checkpoints", "Checkpoints", &[ContextItemKind::Checkpoint]),
    (
        "conversation",
        "Conversation history",
        &[ContextItemKind::Conversation],
    ),
    (
        "current_user",
        "Current user input",
        &[ContextItemKind::User],
    ),
    ("tool_calls", "Tool calls", &[ContextItemKind::ToolCall]),
    (
        "tool_results",
        "Tool results",
        &[ContextItemKind::ToolResult],
    ),
];

fn context_item_details(item: &ModelContextItem) -> Vec<TokenEstimateDetail> {
    if item.kind == ContextItemKind::BaseInstructions && item.source == "opentopia:base" {
        return reconciled_base_prompt_details(item.token_estimate);
    }

    let prompt_module_id = item
        .metadata
        .get("promptModuleId")
        .and_then(serde_json::Value::as_str);
    let skill_name = item
        .metadata
        .get("name")
        .and_then(serde_json::Value::as_str);
    let label = skill_name.or(prompt_module_id).unwrap_or(&item.source);
    let id = prompt_module_id.unwrap_or(&item.source);
    vec![TokenEstimateDetail::leaf(id, label, item.token_estimate)]
}

fn reconciled_base_prompt_details(target: usize) -> Vec<TokenEstimateDetail> {
    let children = BASE_PROMPT_MODULES
        .iter()
        .map(|module| {
            TokenEstimateDetail::leaf(module.id, module.id, estimate_tokens(module.content.trim()))
        })
        .collect::<Vec<_>>();
    reconcile_detail_children(target, children)
}

pub(crate) fn merge_sibling_details(details: Vec<TokenEstimateDetail>) -> Vec<TokenEstimateDetail> {
    let mut merged: Vec<TokenEstimateDetail> = Vec::with_capacity(details.len());
    let mut indexes = HashMap::<String, usize>::with_capacity(details.len());
    for mut detail in details {
        detail.children = merge_sibling_details(detail.children);
        if let Some(index) = indexes.get(&detail.id).copied() {
            let existing = &mut merged[index];
            existing.tokens = existing.tokens.saturating_add(detail.tokens);
            let mut children = std::mem::take(&mut existing.children);
            children.extend(detail.children);
            existing.children = merge_sibling_details(children);
        } else {
            indexes.insert(detail.id.clone(), merged.len());
            merged.push(detail);
        }
    }
    merged
}

pub(crate) fn reconcile_detail_children(
    target: usize,
    mut children: Vec<TokenEstimateDetail>,
) -> Vec<TokenEstimateDetail> {
    let weights = children
        .iter()
        .map(|child| child.tokens)
        .collect::<Vec<_>>();
    for (child, tokens) in children
        .iter_mut()
        .zip(reconcile_token_weights(target, &weights))
    {
        child.tokens = tokens;
    }
    children
}

/// Preserve the exact parent estimate while proportionally attributing assembly
/// differences (separators and tokenizer run boundaries) to child modules.
fn reconcile_token_weights(target: usize, weights: &[usize]) -> Vec<usize> {
    let total = weights.iter().copied().sum::<usize>();
    if total == 0 {
        return vec![0; weights.len()];
    }

    let mut allocated = weights
        .iter()
        .map(|weight| ((*weight as u128 * target as u128) / total as u128) as usize)
        .collect::<Vec<_>>();
    let mut remaining = target.saturating_sub(allocated.iter().sum());
    let mut remainder_order = weights
        .iter()
        .enumerate()
        .map(|(index, weight)| (index, (*weight as u128 * target as u128) % total as u128))
        .collect::<Vec<_>>();
    remainder_order.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    for (index, _) in remainder_order {
        if remaining == 0 {
            break;
        }
        allocated[index] = allocated[index].saturating_add(1);
        remaining -= 1;
    }
    allocated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base_prompt::base_agent_prompt;
    use crate::model_context::{
        ContextCacheScope, ContextRole, ContextSensitivity, ModelContextItem,
    };

    #[test]
    fn reconciled_weights_preserve_parent_total() {
        let result = reconcile_token_weights(11, &[2, 3, 7]);
        assert_eq!(result.iter().sum::<usize>(), 11);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn base_prompt_attribution_uses_the_module_manifest() {
        let item = ModelContextItem::text(
            ContextItemKind::BaseInstructions,
            ContextRole::System,
            "opentopia:base",
            base_agent_prompt(),
            ContextCacheScope::Stable,
            ContextSensitivity::Public,
        );
        let breakdown = TokenEstimateBreakdown::from_context_items(&[item]);
        let root = breakdown
            .details
            .iter()
            .find(|detail| detail.id == "base_instructions")
            .expect("base root");

        assert_eq!(root.children.len(), BASE_PROMPT_MODULES.len());
        assert_eq!(
            root.children
                .iter()
                .map(|child| child.tokens)
                .sum::<usize>(),
            breakdown.base_instructions
        );
        assert_eq!(root.children[0].id, "identity_and_objective");
    }
}
