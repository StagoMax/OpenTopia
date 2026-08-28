use crate::flow::{GraphDefinitionV1, GraphNodeV1};
use crate::workflow_automation::{WorkflowIngressPolicyV1, WorkflowTriggerSpecV1};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use uuid::Uuid;

pub const FLOW_NODE_ACTIVATION_CONFIG_KEY: &str = "activation";

/// Trigger source attached to one Flow Agent node. `AgentFinal` is displayed as
/// a Trigger in the authoring UI, but remains a subscription to the existing
/// Agent completion notification at runtime.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum FlowTriggerSourceV1 {
    /// The unmodified payload that created this FlowRun (`@Flow.input`).
    FlowInput,
    AgentFinal {
        node_id: String,
    },
    Manual,
    Webhook {
        trigger_id: Uuid,
        token_ref: String,
    },
    Schedule {
        trigger_id: Uuid,
        interval_seconds: u32,
        next_fire_at: DateTime<Utc>,
    },
    EventSubscription {
        trigger_id: Uuid,
        source: String,
        event_type: String,
    },
}

impl FlowTriggerSourceV1 {
    pub fn as_workflow_trigger(&self) -> Option<WorkflowTriggerSpecV1> {
        match self {
            Self::FlowInput | Self::AgentFinal { .. } => None,
            Self::Manual => Some(WorkflowTriggerSpecV1::Manual),
            Self::Webhook {
                trigger_id,
                token_ref,
            } => Some(WorkflowTriggerSpecV1::Webhook {
                trigger_id: *trigger_id,
                token_ref: token_ref.clone(),
            }),
            Self::Schedule {
                trigger_id,
                interval_seconds,
                next_fire_at,
            } => Some(WorkflowTriggerSpecV1::Schedule {
                trigger_id: *trigger_id,
                interval_seconds: *interval_seconds,
                next_fire_at: *next_fire_at,
            }),
            Self::EventSubscription {
                trigger_id,
                source,
                event_type,
            } => Some(WorkflowTriggerSpecV1::EventSubscription {
                trigger_id: *trigger_id,
                source: source.clone(),
                event_type: event_type.clone(),
            }),
        }
    }

    fn matches_ingress(&self, trigger: Option<&WorkflowTriggerSpecV1>) -> bool {
        match self {
            Self::FlowInput => true,
            Self::AgentFinal { .. } => false,
            Self::Manual => matches!(trigger, None | Some(WorkflowTriggerSpecV1::Manual)),
            Self::Webhook { trigger_id, .. } => matches!(
                trigger,
                Some(WorkflowTriggerSpecV1::Webhook {
                    trigger_id: current,
                    ..
                }) if current == trigger_id
            ),
            Self::Schedule { trigger_id, .. } => matches!(
                trigger,
                Some(WorkflowTriggerSpecV1::Schedule {
                    trigger_id: current,
                    ..
                }) if current == trigger_id
            ),
            Self::EventSubscription { trigger_id, .. } => matches!(
                trigger,
                Some(WorkflowTriggerSpecV1::EventSubscription {
                    trigger_id: current,
                    ..
                }) if current == trigger_id
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "operator",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum FlowTriggerExpressionV1 {
    Source {
        source: FlowTriggerSourceV1,
    },
    And {
        inputs: Vec<FlowTriggerExpressionV1>,
    },
    Or {
        inputs: Vec<FlowTriggerExpressionV1>,
    },
    Not {
        input: Box<FlowTriggerExpressionV1>,
    },
}

impl FlowTriggerExpressionV1 {
    fn evaluate(
        &self,
        trigger: Option<&WorkflowTriggerSpecV1>,
        completed_nodes: &BTreeSet<String>,
    ) -> bool {
        match self {
            Self::Source { source } => match source {
                FlowTriggerSourceV1::AgentFinal { node_id } => completed_nodes.contains(node_id),
                _ => source.matches_ingress(trigger),
            },
            Self::And { inputs } => {
                !inputs.is_empty()
                    && inputs
                        .iter()
                        .all(|input| input.evaluate(trigger, completed_nodes))
            }
            Self::Or { inputs } => inputs
                .iter()
                .any(|input| input.evaluate(trigger, completed_nodes)),
            Self::Not { input } => !input.evaluate(trigger, completed_nodes),
        }
    }

    fn has_positive_ingress(&self, negated: bool) -> bool {
        match self {
            Self::Source { source } => {
                !negated && !matches!(source, FlowTriggerSourceV1::AgentFinal { .. })
            }
            Self::And { inputs } | Self::Or { inputs } => inputs
                .iter()
                .any(|input| input.has_positive_ingress(negated)),
            Self::Not { input } => input.has_positive_ingress(!negated),
        }
    }

    fn first_external_trigger(&self, negated: bool) -> Option<WorkflowTriggerSpecV1> {
        match self {
            Self::Source { source } if !negated => source.as_workflow_trigger(),
            Self::Source { .. } => None,
            Self::And { inputs } | Self::Or { inputs } => inputs
                .iter()
                .find_map(|input| input.first_external_trigger(negated)),
            Self::Not { input } => input.first_external_trigger(!negated),
        }
    }

    fn contains_external_trigger(&self, trigger: &WorkflowTriggerSpecV1, negated: bool) -> bool {
        match self {
            Self::Source { source } => {
                !negated
                    && !matches!(
                        source,
                        FlowTriggerSourceV1::FlowInput | FlowTriggerSourceV1::AgentFinal { .. }
                    )
                    && source.matches_ingress(Some(trigger))
            }
            Self::And { inputs } | Self::Or { inputs } => inputs
                .iter()
                .any(|input| input.contains_external_trigger(trigger, negated)),
            Self::Not { input } => input.contains_external_trigger(trigger, !negated),
        }
    }

    fn collect_agent_final_sources(&self, target: &mut BTreeSet<String>) {
        match self {
            Self::Source {
                source: FlowTriggerSourceV1::AgentFinal { node_id },
            } => {
                target.insert(node_id.clone());
            }
            Self::Source { .. } => {}
            Self::And { inputs } | Self::Or { inputs } => {
                for input in inputs {
                    input.collect_agent_final_sources(target);
                }
            }
            Self::Not { input } => input.collect_agent_final_sources(target),
        }
    }

    fn validate(&self) -> anyhow::Result<()> {
        match self {
            Self::Source { source } => {
                if let Some(trigger) = source.as_workflow_trigger() {
                    trigger.validate()?;
                }
                if let FlowTriggerSourceV1::AgentFinal { node_id } = source {
                    anyhow::ensure!(!node_id.trim().is_empty(), "Agent Final nodeId is required");
                }
            }
            Self::And { inputs } | Self::Or { inputs } => {
                anyhow::ensure!(
                    inputs.len() >= 2,
                    "AND/OR Trigger expressions require at least two inputs"
                );
                for input in inputs {
                    input.validate()?;
                }
            }
            Self::Not { input } => input.validate()?,
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlowNodeActivationV1 {
    pub expression: FlowTriggerExpressionV1,
    #[serde(default)]
    pub ingress_policy: WorkflowIngressPolicyV1,
}

pub fn parse_node_activation(node: &GraphNodeV1) -> anyhow::Result<Option<FlowNodeActivationV1>> {
    let Some(value) = node.config.get(FLOW_NODE_ACTIVATION_CONFIG_KEY) else {
        return Ok(None);
    };
    let activation = serde_json::from_value::<FlowNodeActivationV1>(value.clone())?;
    activation.expression.validate()?;
    Ok(Some(activation))
}

pub fn activation_agent_final_sources(node: &GraphNodeV1) -> anyhow::Result<BTreeSet<String>> {
    let Some(activation) = parse_node_activation(node)? else {
        return Ok(BTreeSet::new());
    };
    let mut sources = BTreeSet::new();
    activation
        .expression
        .collect_agent_final_sources(&mut sources);
    Ok(sources)
}

pub fn activation_root_node_ids(graph: &GraphDefinitionV1) -> BTreeSet<String> {
    graph
        .nodes
        .iter()
        .filter_map(|node| {
            parse_node_activation(node)
                .ok()
                .flatten()
                .filter(|activation| activation.expression.has_positive_ingress(false))
                .map(|_| node.id.clone())
        })
        .collect()
}

pub fn default_graph_trigger(graph: &GraphDefinitionV1) -> WorkflowTriggerSpecV1 {
    let entry = graph
        .nodes
        .iter()
        .find(|node| node.id == graph.entry_node_id);
    entry
        .and_then(|node| parse_node_activation(node).ok().flatten())
        .and_then(|activation| activation.expression.first_external_trigger(false))
        .or_else(|| {
            graph.nodes.iter().find_map(|node| {
                parse_node_activation(node)
                    .ok()
                    .flatten()
                    .and_then(|activation| activation.expression.first_external_trigger(false))
            })
        })
        .unwrap_or(WorkflowTriggerSpecV1::Manual)
}

pub fn default_graph_ingress_policy(graph: &GraphDefinitionV1) -> WorkflowIngressPolicyV1 {
    graph
        .nodes
        .iter()
        .find(|node| node.id == graph.entry_node_id)
        .and_then(|node| parse_node_activation(node).ok().flatten())
        .map(|activation| activation.ingress_policy)
        .unwrap_or_default()
}

pub fn graph_ingress_policy_for_trigger(
    graph: &GraphDefinitionV1,
    trigger: &WorkflowTriggerSpecV1,
) -> WorkflowIngressPolicyV1 {
    graph
        .nodes
        .iter()
        .filter_map(|node| parse_node_activation(node).ok().flatten())
        .find(|activation| {
            activation
                .expression
                .contains_external_trigger(trigger, false)
        })
        .map(|activation| activation.ingress_policy)
        .unwrap_or_default()
}

pub fn initial_ready_nodes(
    graph: &GraphDefinitionV1,
    trigger: Option<&WorkflowTriggerSpecV1>,
) -> Vec<String> {
    let completed = BTreeSet::new();
    let matched = graph
        .nodes
        .iter()
        .filter_map(|node| {
            let activation = parse_node_activation(node).ok().flatten()?;
            (activation.expression.has_positive_ingress(false)
                && activation.expression.evaluate(trigger, &completed))
            .then(|| node.id.clone())
        })
        .collect::<Vec<_>>();
    let entry_has_activation = graph
        .nodes
        .iter()
        .find(|node| node.id == graph.entry_node_id)
        .is_some_and(|node| parse_node_activation(node).ok().flatten().is_some());
    if matched.is_empty() && !entry_has_activation {
        vec![graph.entry_node_id.clone()]
    } else {
        matched
    }
}

pub fn node_activation_ready(
    node: &GraphNodeV1,
    trigger: Option<&WorkflowTriggerSpecV1>,
    completed_nodes: impl IntoIterator<Item = String>,
) -> anyhow::Result<Option<bool>> {
    let Some(activation) = parse_node_activation(node)? else {
        return Ok(None);
    };
    let completed = completed_nodes.into_iter().collect::<BTreeSet<_>>();
    Ok(Some(activation.expression.evaluate(trigger, &completed)))
}

pub fn validate_graph_activations(graph: &GraphDefinitionV1) -> Vec<(String, String)> {
    let node_ids = graph
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    let edges = graph
        .edges
        .iter()
        .map(|edge| (edge.from.as_str(), edge.to.as_str()))
        .collect::<HashSet<_>>();
    let mut issues = Vec::new();
    for node in &graph.nodes {
        let activation = match parse_node_activation(node) {
            Ok(Some(activation)) => activation,
            Ok(None) => continue,
            Err(error) => {
                issues.push((node.id.clone(), error.to_string()));
                continue;
            }
        };
        let mut sources = BTreeSet::new();
        activation
            .expression
            .collect_agent_final_sources(&mut sources);
        for source in sources {
            if !node_ids.contains(source.as_str()) {
                issues.push((
                    node.id.clone(),
                    format!("Trigger references missing Agent Final '{source}'"),
                ));
            } else if !edges.contains(&(source.as_str(), node.id.as_str())) {
                issues.push((
                    node.id.clone(),
                    format!(
                        "Trigger subscribes to Agent Final '{source}' without a matching graph edge"
                    ),
                ));
            }
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enterprise::DataClassification;
    use crate::flow::{GraphEdgeV1, GraphNodeKindV1};
    use serde_json::json;

    fn node(id: &str, activation: serde_json::Value) -> GraphNodeV1 {
        GraphNodeV1 {
            id: id.to_string(),
            label: id.to_string(),
            kind: GraphNodeKindV1::Agent,
            config: json!({"activation": activation}),
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn internal_final_subscription_supports_and_not() {
        let target = node(
            "target",
            json!({
                "expression": {
                    "operator": "and",
                    "inputs": [
                        {"operator": "source", "source": {"kind": "agent_final", "nodeId": "a"}},
                        {"operator": "not", "input": {"operator": "source", "source": {"kind": "agent_final", "nodeId": "blocked"}}}
                    ]
                },
                "ingressPolicy": "immediate"
            }),
        );
        assert_eq!(
            node_activation_ready(&target, None, ["a".to_string()]).unwrap(),
            Some(true)
        );
        assert_eq!(
            node_activation_ready(&target, None, ["a".to_string(), "blocked".to_string()]).unwrap(),
            Some(false)
        );
    }

    #[test]
    fn graph_selects_the_node_whose_external_trigger_started_the_run() {
        let trigger_id = Uuid::new_v4();
        let first = node(
            "first",
            json!({
                "expression": {"operator": "source", "source": {"kind": "manual"}},
                "ingressPolicy": "require_review"
            }),
        );
        let webhook = node(
            "webhook",
            json!({
                "expression": {"operator": "source", "source": {
                    "kind": "webhook", "triggerId": trigger_id,
                    "tokenRef": "env:FLOW_TOKEN"
                }},
                "ingressPolicy": "immediate"
            }),
        );
        let graph = GraphDefinitionV1 {
            schema_version: 1,
            entry_node_id: "first".to_string(),
            nodes: vec![first, webhook],
            edges: Vec::new(),
        };
        let trigger = WorkflowTriggerSpecV1::Webhook {
            trigger_id,
            token_ref: "env:FLOW_TOKEN".to_string(),
        };
        assert_eq!(initial_ready_nodes(&graph, Some(&trigger)), vec!["webhook"]);
    }

    #[test]
    fn configured_entry_does_not_fall_back_for_an_unrelated_trigger() {
        let graph = GraphDefinitionV1 {
            schema_version: 1,
            entry_node_id: "entry".to_string(),
            nodes: vec![node(
                "entry",
                json!({
                    "expression": {"operator": "source", "source": {
                        "kind": "webhook", "triggerId": Uuid::new_v4(),
                        "tokenRef": "env:FLOW_TOKEN"
                    }},
                    "ingressPolicy": "immediate"
                }),
            )],
            edges: Vec::new(),
        };
        let unrelated = WorkflowTriggerSpecV1::EventSubscription {
            trigger_id: Uuid::new_v4(),
            source: "crm".to_string(),
            event_type: "record.updated".to_string(),
        };

        assert!(initial_ready_nodes(&graph, Some(&unrelated)).is_empty());
    }

    #[test]
    fn trigger_uses_the_policy_of_its_configured_agent_node() {
        let trigger_id = Uuid::new_v4();
        let graph = GraphDefinitionV1 {
            schema_version: 1,
            entry_node_id: "entry".to_string(),
            nodes: vec![node(
                "entry",
                json!({
                    "expression": {"operator": "source", "source": {
                        "kind": "event_subscription", "triggerId": trigger_id,
                        "source": "mail", "eventType": "message.received"
                    }},
                    "ingressPolicy": "require_review"
                }),
            )],
            edges: Vec::new(),
        };
        let trigger = WorkflowTriggerSpecV1::EventSubscription {
            trigger_id,
            source: "mail".to_string(),
            event_type: "message.received".to_string(),
        };

        assert_eq!(
            graph_ingress_policy_for_trigger(&graph, &trigger),
            WorkflowIngressPolicyV1::RequireReview
        );
    }

    #[test]
    fn activation_validation_accepts_a_matching_subscription_edge() {
        let graph = GraphDefinitionV1 {
            schema_version: 1,
            entry_node_id: "a".to_string(),
            nodes: vec![
                node(
                    "a",
                    json!({
                        "expression": {"operator": "source", "source": {"kind": "manual"}},
                        "ingressPolicy": "immediate"
                    }),
                ),
                node(
                    "b",
                    json!({
                        "expression": {"operator": "source", "source": {"kind": "agent_final", "nodeId": "a"}},
                        "ingressPolicy": "immediate"
                    }),
                ),
            ],
            edges: vec![GraphEdgeV1 {
                from: "a".to_string(),
                to: "b".to_string(),
                condition: None,
                allowed_fields: BTreeSet::new(),
                data_classification: DataClassification::Internal,
                on_error: None,
                loop_policy: None,
            }],
        };
        assert!(validate_graph_activations(&graph).is_empty());
    }
}
