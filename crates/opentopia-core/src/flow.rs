use crate::enterprise::{
    AgentRiskClassV1, CapabilityProjection, DataClassification, ENTERPRISE_SCHEMA_VERSION_V1,
};
use crate::flow_activation::{activation_root_node_ids, validate_graph_activations};
use crate::model_context::content_fingerprint;
use crate::workflow_state::validate_graph_state_writes;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use uuid::Uuid;

pub const MAX_FLOW_NODES: usize = 128;
pub const MAX_FLOW_LOOP_ITERATIONS: u32 = 50;
pub const MAX_FLOW_DURATION_SECONDS: u64 = 86_400;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum FlowSourceV1 {
    NaturalLanguage { description: String },
    RunTrace { run_id: Uuid, trace_hash: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowDraftStatusV1 {
    Drafting,
    Reviewing,
    Validating,
    ReadyToPublish,
    Published,
}

impl FlowDraftStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Drafting => "drafting",
            Self::Reviewing => "reviewing",
            Self::Validating => "validating",
            Self::ReadyToPublish => "ready_to_publish",
            Self::Published => "published",
        }
    }
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeKindV1 {
    Agent,
    Skill,
    Tool,
    Condition,
    Validator,
    Approval,
    Join,
    Loop,
    Output,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GraphNodeV1 {
    pub id: String,
    pub label: String,
    pub kind: GraphNodeKindV1,
    #[serde(default)]
    pub config: Value,
    #[serde(default = "object_schema")]
    pub input_schema: Value,
    #[serde(default = "object_schema")]
    pub output_schema: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoopExhaustionActionV1 {
    RequireHuman,
    ReturnPartial,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphLoopPolicyV1 {
    pub max_iterations: u32,
    pub continue_condition: String,
    pub on_exhausted: LoopExhaustionActionV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdgeV1 {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(default)]
    pub allowed_fields: BTreeSet<String>,
    #[serde(default = "default_classification")]
    pub data_classification: DataClassification,
    #[serde(default)]
    pub on_error: Option<String>,
    #[serde(default)]
    pub loop_policy: Option<GraphLoopPolicyV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GraphDefinitionV1 {
    pub schema_version: u16,
    pub entry_node_id: String,
    pub nodes: Vec<GraphNodeV1>,
    pub edges: Vec<GraphEdgeV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlowBudgetV1 {
    pub max_node_executions: u32,
    pub max_tool_calls: u32,
    pub max_duration_seconds: u64,
    pub max_loop_iterations: u32,
}

impl Default for FlowBudgetV1 {
    fn default() -> Self {
        Self {
            max_node_executions: 100,
            max_tool_calls: 60,
            max_duration_seconds: 3_600,
            max_loop_iterations: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowSpecV1 {
    pub flow_id: String,
    pub name: String,
    pub description: String,
    pub owner: String,
    #[serde(default)]
    pub categories: BTreeSet<String>,
    pub source: FlowSourceV1,
    #[serde(default = "object_schema")]
    pub input_schema: Value,
    #[serde(default = "object_schema")]
    pub output_schema: Value,
    pub graph: GraphDefinitionV1,
    #[serde(default)]
    pub requested_capabilities: CapabilityProjection,
    #[serde(default)]
    pub budget: FlowBudgetV1,
    pub risk_class: AgentRiskClassV1,
    #[serde(default)]
    pub pending_decisions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowValidationSeverityV1 {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlowValidationIssueV1 {
    pub severity: FlowValidationSeverityV1,
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub edge_index: Option<usize>,
    pub remediation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlowValidationReportV1 {
    pub valid: bool,
    pub issues: Vec<FlowValidationIssueV1>,
    pub validated_at: DateTime<Utc>,
}

impl FlowValidationReportV1 {
    pub fn error_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.severity == FlowValidationSeverityV1::Error)
            .count()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowDraftV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub thread_id: Uuid,
    pub revision: u32,
    pub status: FlowDraftStatusV1,
    pub spec: FlowSpecV1,
    pub effective_capabilities: CapabilityProjection,
    pub content_hash: String,
    #[serde(default)]
    pub last_validation: Option<FlowValidationReportV1>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl FlowDraftV1 {
    pub fn new(
        thread_id: Uuid,
        mut spec: FlowSpecV1,
        execution_capabilities: &CapabilityProjection,
    ) -> Self {
        normalize_flow_spec(&mut spec);
        let now = Utc::now();
        let content_hash = flow_content_hash(&spec);
        Self {
            schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
            id: Uuid::new_v4(),
            thread_id,
            revision: 1,
            status: FlowDraftStatusV1::Reviewing,
            effective_capabilities: spec
                .requested_capabilities
                .intersect(execution_capabilities),
            spec,
            content_hash,
            last_validation: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn replace_spec(
        &mut self,
        mut spec: FlowSpecV1,
        execution_capabilities: &CapabilityProjection,
    ) {
        normalize_flow_spec(&mut spec);
        self.revision = self.revision.saturating_add(1);
        self.status = FlowDraftStatusV1::Reviewing;
        self.effective_capabilities = spec
            .requested_capabilities
            .intersect(execution_capabilities);
        self.content_hash = flow_content_hash(&spec);
        self.spec = spec;
        self.last_validation = None;
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowDefinitionV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub flow_id: String,
    pub name: String,
    pub version: u32,
    pub owner: String,
    pub description: String,
    pub categories: BTreeSet<String>,
    pub source: FlowSourceV1,
    pub graph: GraphDefinitionV1,
    pub input_schema: Value,
    pub output_schema: Value,
    pub capabilities: CapabilityProjection,
    pub budget: FlowBudgetV1,
    pub risk_class: AgentRiskClassV1,
    pub content_hash: String,
    pub published_at: DateTime<Utc>,
    pub published_by: String,
}

impl FlowDefinitionV1 {
    pub fn to_spec(&self) -> FlowSpecV1 {
        FlowSpecV1 {
            flow_id: self.flow_id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            owner: self.owner.clone(),
            categories: self.categories.clone(),
            source: self.source.clone(),
            input_schema: self.input_schema.clone(),
            output_schema: self.output_schema.clone(),
            graph: self.graph.clone(),
            requested_capabilities: self.capabilities.clone(),
            budget: self.budget.clone(),
            risk_class: self.risk_class,
            pending_decisions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowTrialStatusV1 {
    Passed,
    Failed,
}

impl FlowTrialStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FlowSimulationStepV1 {
    pub order: u32,
    pub node_id: String,
    pub harness_target: String,
    pub bounded_by: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowTrialV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub draft_id: Uuid,
    pub draft_revision: u32,
    pub status: FlowTrialStatusV1,
    pub input: Value,
    pub steps: Vec<FlowSimulationStepV1>,
    pub report: FlowValidationReportV1,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HarnessNodeTargetV1 {
    AgentCore,
    AgentRunScheduler,
    SkillRuntime,
    ToolRegistry,
    RuntimeCondition,
    RuntimeValidator,
    RuntimeApproval,
    RuntimeJoin,
    RuntimeLoop,
    RuntimeOutput,
}

impl HarnessNodeTargetV1 {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AgentCore => "agent_core",
            Self::AgentRunScheduler => "agent_run_scheduler",
            Self::SkillRuntime => "skill_runtime",
            Self::ToolRegistry => "tool_registry",
            Self::RuntimeCondition => "runtime_condition",
            Self::RuntimeValidator => "runtime_validator",
            Self::RuntimeApproval => "runtime_approval",
            Self::RuntimeJoin => "runtime_join",
            Self::RuntimeLoop => "runtime_loop",
            Self::RuntimeOutput => "runtime_output",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompiledFlowNodeV1 {
    pub node_id: String,
    pub target: HarnessNodeTargetV1,
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompiledFlowPlanV1 {
    pub schema_version: u16,
    pub flow_id: String,
    pub content_hash: String,
    pub entry_node_id: String,
    pub nodes: Vec<CompiledFlowNodeV1>,
    pub edges: Vec<GraphEdgeV1>,
    pub budget: FlowBudgetV1,
}

pub fn normalize_flow_spec(spec: &mut FlowSpecV1) {
    spec.flow_id = spec.flow_id.trim().to_lowercase().replace(' ', "-");
    spec.name = spec.name.trim().to_string();
    spec.description = spec.description.trim().to_string();
    spec.owner = spec.owner.trim().to_string();
    spec.graph.entry_node_id = spec.graph.entry_node_id.trim().to_string();
    for node in &mut spec.graph.nodes {
        node.id = node.id.trim().to_string();
        node.label = node.label.trim().to_string();
    }
}

pub fn flow_content_hash(spec: &FlowSpecV1) -> String {
    let bytes = serde_json::to_vec(spec).unwrap_or_default();
    content_fingerprint(&bytes)
}

pub fn validate_flow_spec(
    spec: &FlowSpecV1,
    execution_capabilities: &CapabilityProjection,
) -> FlowValidationReportV1 {
    let mut issues = Vec::new();
    let mut error = |code: &str,
                     message: String,
                     node_id: Option<String>,
                     edge_index: Option<usize>,
                     remediation: &str| {
        issues.push(FlowValidationIssueV1 {
            severity: FlowValidationSeverityV1::Error,
            code: code.to_string(),
            message,
            node_id,
            edge_index,
            remediation: remediation.to_string(),
        });
    };

    if spec.flow_id.is_empty() || spec.name.is_empty() || spec.owner.is_empty() {
        error(
            "flow.identity.required",
            "flowId, name and owner are required".to_string(),
            None,
            None,
            "Provide a stable flowId, a reviewable name, and an accountable owner.",
        );
    }
    if !spec.pending_decisions.is_empty() {
        error(
            "flow.decisions.pending",
            format!(
                "{} design decision(s) still require human confirmation",
                spec.pending_decisions.len()
            ),
            None,
            None,
            "Resolve or explicitly remove every pending decision before publication.",
        );
    }
    if spec.graph.nodes.is_empty() || spec.graph.nodes.len() > MAX_FLOW_NODES {
        error(
            "graph.node_count.invalid",
            format!("graph must contain between 1 and {MAX_FLOW_NODES} nodes"),
            None,
            None,
            "Split very large workflows into reusable child Flows.",
        );
    }
    if spec.budget.max_node_executions == 0
        || spec.budget.max_tool_calls == 0
        || spec.budget.max_duration_seconds == 0
        || spec.budget.max_duration_seconds > MAX_FLOW_DURATION_SECONDS
        || spec.budget.max_loop_iterations == 0
        || spec.budget.max_loop_iterations > MAX_FLOW_LOOP_ITERATIONS
    {
        error(
            "flow.budget.invalid",
            "all budgets must be positive and within platform limits".to_string(),
            None,
            None,
            "Set explicit node, tool, duration, and loop budgets within platform limits.",
        );
    }

    let nodes: BTreeMap<&str, &GraphNodeV1> = spec
        .graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    if nodes.len() != spec.graph.nodes.len() {
        error(
            "graph.node_id.duplicate",
            "node ids must be unique".to_string(),
            None,
            None,
            "Assign a unique stable id to every node.",
        );
    }
    if !nodes.contains_key(spec.graph.entry_node_id.as_str()) {
        error(
            "graph.entry.missing",
            format!("entry node '{}' does not exist", spec.graph.entry_node_id),
            None,
            None,
            "Set entryNodeId to exactly one existing node.",
        );
    }
    let output_nodes = spec
        .graph
        .nodes
        .iter()
        .filter(|node| node.kind == GraphNodeKindV1::Output)
        .count();
    if output_nodes == 0 {
        error(
            "graph.output.missing",
            "graph requires at least one output node".to_string(),
            None,
            None,
            "Add an output node for every terminal path.",
        );
    }

    let mut adjacency: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (index, edge) in spec.graph.edges.iter().enumerate() {
        if !nodes.contains_key(edge.from.as_str()) || !nodes.contains_key(edge.to.as_str()) {
            error(
                "graph.edge.reference_missing",
                format!("edge {} references a missing node", index),
                None,
                Some(index),
                "Make both edge endpoints reference existing node ids.",
            );
            continue;
        }
        adjacency
            .entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
        if let Some(condition) = &edge.condition {
            if !safe_condition(condition) {
                error(
                    "graph.condition.unsafe",
                    "conditions must use the bounded expression subset".to_string(),
                    None,
                    Some(index),
                    "Use field comparisons and boolean operators; do not embed code or calls.",
                );
            }
        }
        if let Some(policy) = &edge.loop_policy {
            if policy.max_iterations == 0
                || policy.max_iterations > spec.budget.max_loop_iterations
                || policy.max_iterations > MAX_FLOW_LOOP_ITERATIONS
                || policy.continue_condition.trim().is_empty()
                || !safe_condition(&policy.continue_condition)
            {
                error(
                    "graph.loop.unbounded",
                    "loop edge exceeds the Flow budget or has no safe termination condition"
                        .to_string(),
                    None,
                    Some(index),
                    "Declare maxIterations, a structured continueCondition, and onExhausted.",
                );
            }
        }
        let source_schema = &nodes[edge.from.as_str()].output_schema;
        let target_schema = &nodes[edge.to.as_str()].input_schema;
        if !schemas_compatible(source_schema, target_schema, &edge.allowed_fields) {
            error(
                "graph.schema.incompatible",
                format!("edge {} does not satisfy the target input schema", index),
                None,
                Some(index),
                "Align node schemas or whitelist only fields accepted by the target.",
            );
        }
        let source_classification = nodes[edge.from.as_str()]
            .config
            .get("outputClassification")
            .and_then(Value::as_str)
            .and_then(parse_classification)
            .unwrap_or(DataClassification::Internal);
        if edge.data_classification < source_classification {
            error(
                "graph.data_classification.downgrade",
                format!("edge {} downgrades its source data classification", index),
                None,
                Some(index),
                "Keep or raise classification across an edge; declassification requires a separate governed process.",
            );
        }
    }

    for (node_id, message) in validate_graph_activations(&spec.graph) {
        error(
            "graph.trigger.invalid",
            message,
            Some(node_id),
            None,
            "Make every Trigger source valid and keep Agent Final subscriptions aligned with graph edges.",
        );
    }

    let mut reachable = BTreeSet::new();
    let activation_roots = activation_root_node_ids(&spec.graph);
    let mut queue = if activation_roots.is_empty() {
        VecDeque::from([spec.graph.entry_node_id.as_str()])
    } else {
        activation_roots
            .iter()
            .map(String::as_str)
            .collect::<VecDeque<_>>()
    };
    while let Some(id) = queue.pop_front() {
        if !reachable.insert(id) {
            continue;
        }
        if let Some(next) = adjacency.get(id) {
            queue.extend(next.iter().copied());
        }
    }
    for node in &spec.graph.nodes {
        if !reachable.contains(node.id.as_str()) {
            error(
                "graph.node.unreachable",
                format!("node '{}' is unreachable", node.id),
                Some(node.id.clone()),
                None,
                "Connect the node from the entry path or remove it.",
            );
        }
        if node.kind == GraphNodeKindV1::Output
            && adjacency
                .get(node.id.as_str())
                .is_some_and(|outgoing| !outgoing.is_empty())
        {
            error(
                "graph.output.not_terminal",
                format!("output node '{}' has outgoing edges", node.id),
                Some(node.id.clone()),
                None,
                "Remove outgoing edges so every output node is terminal.",
            );
        }
        validate_node(node, execution_capabilities, &mut error);
    }

    for (node_id, message) in validate_graph_state_writes(
        spec.graph
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), &node.config)),
    ) {
        error(
            "graph.state_channel.invalid",
            message,
            Some(node_id),
            None,
            "Use a stable channel name and one reducer contract; replace channels must have a single writer.",
        );
    }

    if contains_unbounded_cycle(&spec.graph) {
        error(
            "graph.cycle.unbounded",
            "every cycle must include a bounded loop edge".to_string(),
            None,
            None,
            "Mark the feedback edge with a loopPolicy and an exhaustion action.",
        );
    }

    validate_capability_scope(
        &spec.requested_capabilities,
        execution_capabilities,
        &mut error,
    );

    if matches!(
        spec.risk_class,
        AgentRiskClassV1::High | AgentRiskClassV1::Critical
    ) {
        let has_validator = spec
            .graph
            .nodes
            .iter()
            .any(|node| node.kind == GraphNodeKindV1::Validator);
        let has_approval = spec
            .graph
            .nodes
            .iter()
            .any(|node| node.kind == GraphNodeKindV1::Approval);
        if !has_validator || !has_approval {
            error(
                "flow.risk.gate_missing",
                "high-risk Flows require both validator and approval nodes".to_string(),
                None,
                None,
                "Add deterministic validation and an explicit human approval gate.",
            );
        }
    }

    let valid = !issues
        .iter()
        .any(|issue| issue.severity == FlowValidationSeverityV1::Error);
    FlowValidationReportV1 {
        valid,
        issues,
        validated_at: Utc::now(),
    }
}

fn validate_node(
    node: &GraphNodeV1,
    execution_capabilities: &CapabilityProjection,
    error: &mut impl FnMut(&str, String, Option<String>, Option<usize>, &str),
) {
    let reference = node.config.get("reference").and_then(Value::as_str);
    match node.kind {
        GraphNodeKindV1::Tool => match reference {
            Some(name) if execution_capabilities.allows_tool(name) => {
                let side_effect = node
                    .config
                    .get("sideEffect")
                    .and_then(Value::as_str)
                    .unwrap_or("none");
                if !matches!(side_effect, "none" | "read_only")
                    && node.config.get("recoveryPolicy").is_none()
                {
                    error(
                        "node.tool.recovery_required",
                        format!("side-effecting tool '{name}' has no recoveryPolicy"),
                        Some(node.id.clone()),
                        None,
                        "Declare idempotency, retry safety, compensation, or require-human recovery behavior.",
                    );
                }
            }
            Some(name) => error(
                "node.tool.not_visible",
                format!("tool '{name}' is not visible in the active ExecutionContext"),
                Some(node.id.clone()),
                None,
                "Choose a visible tool or update the Agent template outside this Flow.",
            ),
            None => error(
                "node.tool.reference_required",
                "tool nodes require config.reference".to_string(),
                Some(node.id.clone()),
                None,
                "Pin the exact tool name that the existing ToolRegistry exposes.",
            ),
        },
        GraphNodeKindV1::Skill => match reference {
            Some(id) if execution_capabilities.allows_skill(id) => {}
            Some(id) => error(
                "node.skill.not_visible",
                format!("Skill '{id}' is not visible in the active ExecutionContext"),
                Some(node.id.clone()),
                None,
                "Choose a visible Skill or update the Agent template outside this Flow.",
            ),
            None => error(
                "node.skill.reference_required",
                "Skill nodes require config.reference".to_string(),
                Some(node.id.clone()),
                None,
                "Pin the exact Skill id and version.",
            ),
        },
        GraphNodeKindV1::Agent => {
            if reference.is_none()
                || node
                    .config
                    .get("templateVersion")
                    .and_then(Value::as_u64)
                    .is_none()
            {
                error(
                    "node.agent.version_required",
                    "Agent nodes require config.reference and a pinned templateVersion".to_string(),
                    Some(node.id.clone()),
                    None,
                    "Reference a published Agent template and pin its immutable version.",
                );
            }
        }
        GraphNodeKindV1::Loop => {
            if node.config.get("feedbackSchema").is_none() {
                error(
                    "node.loop.feedback_required",
                    "loop nodes require a structured feedbackSchema".to_string(),
                    Some(node.id.clone()),
                    None,
                    "Describe the test/review feedback returned to the rework node.",
                );
            }
        }
        _ => {}
    }
}

fn validate_capability_scope(
    requested: &CapabilityProjection,
    available: &CapabilityProjection,
    error: &mut impl FnMut(&str, String, Option<String>, Option<usize>, &str),
) {
    if !requested.is_subset_of(available) {
        error(
            "flow.capability.expansion",
            "requested capabilities exceed the active ExecutionContext".to_string(),
            None,
            None,
            "Reduce requestedCapabilities; a Flow cannot grant tools, Skills, plugins, data, or workspace access.",
        );
    }
}

fn safe_condition(condition: &str) -> bool {
    !condition.trim().is_empty()
        && condition.len() <= 512
        && ![";", "{", "}", "=>", "function", "import", "eval", "exec("]
            .iter()
            .any(|token| condition.contains(token))
}

fn schema_type(schema: &Value) -> Option<&str> {
    schema.get("type").and_then(Value::as_str)
}

fn parse_classification(value: &str) -> Option<DataClassification> {
    match value {
        "public" => Some(DataClassification::Public),
        "internal" => Some(DataClassification::Internal),
        "confidential" => Some(DataClassification::Confidential),
        "restricted" => Some(DataClassification::Restricted),
        _ => None,
    }
}

fn schemas_compatible(source: &Value, target: &Value, allowed_fields: &BTreeSet<String>) -> bool {
    if matches!((schema_type(source), schema_type(target)), (Some(left), Some(right)) if left != right)
    {
        return false;
    }
    let source_properties = source.get("properties").and_then(Value::as_object);
    let target_required = target.get("required").and_then(Value::as_array);
    match (source_properties, target_required) {
        (Some(properties), Some(required)) => required.iter().all(|field| {
            field.as_str().is_some_and(|field| {
                properties.contains_key(field)
                    && (allowed_fields.is_empty() || allowed_fields.contains(field))
            })
        }),
        _ => true,
    }
}

fn contains_unbounded_cycle(graph: &GraphDefinitionV1) -> bool {
    fn visit<'a>(
        id: &'a str,
        adjacency: &BTreeMap<&'a str, Vec<(&'a str, bool)>>,
        visiting: &mut BTreeSet<&'a str>,
        visited: &mut BTreeSet<&'a str>,
    ) -> bool {
        if !visiting.insert(id) {
            return true;
        }
        if visited.contains(id) {
            visiting.remove(id);
            return false;
        }
        if let Some(next) = adjacency.get(id) {
            for (target, bounded) in next {
                if *bounded {
                    continue;
                }
                if visit(target, adjacency, visiting, visited) {
                    return true;
                }
            }
        }
        visiting.remove(id);
        visited.insert(id);
        false
    }

    let mut adjacency: BTreeMap<&str, Vec<(&str, bool)>> = BTreeMap::new();
    for edge in &graph.edges {
        adjacency
            .entry(edge.from.as_str())
            .or_default()
            .push((edge.to.as_str(), edge.loop_policy.is_some()));
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    graph.nodes.iter().any(|node| {
        !visited.contains(node.id.as_str())
            && visit(node.id.as_str(), &adjacency, &mut visiting, &mut visited)
    })
}

pub fn compile_flow(
    spec: &FlowSpecV1,
    execution_capabilities: &CapabilityProjection,
) -> Result<CompiledFlowPlanV1, FlowValidationReportV1> {
    let report = validate_flow_spec(spec, execution_capabilities);
    if !report.valid {
        return Err(report);
    }
    let nodes = spec
        .graph
        .nodes
        .iter()
        .map(|node| {
            let target = match node.kind {
                GraphNodeKindV1::Agent => {
                    if node.config.get("delegate").and_then(Value::as_bool) == Some(true) {
                        HarnessNodeTargetV1::AgentRunScheduler
                    } else {
                        HarnessNodeTargetV1::AgentCore
                    }
                }
                GraphNodeKindV1::Skill => HarnessNodeTargetV1::SkillRuntime,
                GraphNodeKindV1::Tool => HarnessNodeTargetV1::ToolRegistry,
                GraphNodeKindV1::Condition => HarnessNodeTargetV1::RuntimeCondition,
                GraphNodeKindV1::Validator => HarnessNodeTargetV1::RuntimeValidator,
                GraphNodeKindV1::Approval => HarnessNodeTargetV1::RuntimeApproval,
                GraphNodeKindV1::Join => HarnessNodeTargetV1::RuntimeJoin,
                GraphNodeKindV1::Loop => HarnessNodeTargetV1::RuntimeLoop,
                GraphNodeKindV1::Output => HarnessNodeTargetV1::RuntimeOutput,
            };
            CompiledFlowNodeV1 {
                node_id: node.id.clone(),
                target,
                reference: node
                    .config
                    .get("reference")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            }
        })
        .collect();
    Ok(CompiledFlowPlanV1 {
        schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
        flow_id: spec.flow_id.clone(),
        content_hash: flow_content_hash(spec),
        entry_node_id: spec.graph.entry_node_id.clone(),
        nodes,
        edges: spec.graph.edges.clone(),
        budget: spec.budget.clone(),
    })
}

pub fn simulate_flow(
    draft: &FlowDraftV1,
    input: Value,
    execution_capabilities: &CapabilityProjection,
) -> FlowTrialV1 {
    let report = validate_flow_spec(&draft.spec, execution_capabilities);
    let mut steps = Vec::new();
    if let Ok(plan) = compile_flow(&draft.spec, execution_capabilities) {
        for (index, node) in plan.nodes.iter().enumerate() {
            let bounded_by = draft
                .spec
                .graph
                .edges
                .iter()
                .find(|edge| edge.to == node.node_id)
                .and_then(|edge| edge.loop_policy.as_ref())
                .map(|policy| policy.max_iterations);
            steps.push(FlowSimulationStepV1 {
                order: index as u32,
                node_id: node.node_id.clone(),
                harness_target: node.target.as_str().to_string(),
                bounded_by,
            });
        }
    }
    FlowTrialV1 {
        schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
        id: Uuid::new_v4(),
        draft_id: draft.id,
        draft_revision: draft.revision,
        status: if report.valid {
            FlowTrialStatusV1::Passed
        } else {
            FlowTrialStatusV1::Failed
        },
        input,
        steps,
        report,
        created_at: Utc::now(),
    }
}

pub fn definition_from_draft(
    draft: &FlowDraftV1,
    version: u32,
    published_by: impl Into<String>,
) -> FlowDefinitionV1 {
    FlowDefinitionV1 {
        schema_version: ENTERPRISE_SCHEMA_VERSION_V1,
        id: Uuid::new_v4(),
        flow_id: draft.spec.flow_id.clone(),
        name: draft.spec.name.clone(),
        version,
        owner: draft.spec.owner.clone(),
        description: draft.spec.description.clone(),
        categories: draft.spec.categories.clone(),
        source: draft.spec.source.clone(),
        graph: draft.spec.graph.clone(),
        input_schema: draft.spec.input_schema.clone(),
        output_schema: draft.spec.output_schema.clone(),
        capabilities: draft.effective_capabilities.clone(),
        budget: draft.spec.budget.clone(),
        risk_class: draft.spec.risk_class,
        content_hash: draft.content_hash.clone(),
        published_at: Utc::now(),
        published_by: published_by.into(),
    }
}

fn object_schema() -> Value {
    json!({"type": "object"})
}

fn default_classification() -> DataClassification {
    DataClassification::Internal
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: GraphNodeKindV1, config: Value) -> GraphNodeV1 {
        GraphNodeV1 {
            id: id.to_string(),
            label: id.to_string(),
            kind,
            config,
            input_schema: object_schema(),
            output_schema: object_schema(),
        }
    }

    fn edge(from: &str, to: &str) -> GraphEdgeV1 {
        GraphEdgeV1 {
            from: from.to_string(),
            to: to.to_string(),
            condition: None,
            allowed_fields: BTreeSet::new(),
            data_classification: DataClassification::Internal,
            on_error: None,
            loop_policy: None,
        }
    }

    fn valid_spec() -> (FlowSpecV1, CapabilityProjection) {
        let capabilities = CapabilityProjection::only_tools(["shell"]);
        let mut feedback = edge("validate", "build");
        feedback.condition = Some("result.passed == false".to_string());
        feedback.loop_policy = Some(GraphLoopPolicyV1 {
            max_iterations: 3,
            continue_condition: "result.passed == false".to_string(),
            on_exhausted: LoopExhaustionActionV1::RequireHuman,
        });
        (
            FlowSpecV1 {
                flow_id: "delivery-flow".to_string(),
                name: "Delivery Flow".to_string(),
                description: "Build, validate and return evidence".to_string(),
                owner: "platform".to_string(),
                categories: BTreeSet::from(["engineering".to_string()]),
                source: FlowSourceV1::NaturalLanguage {
                    description: "develop, test, rework, report".to_string(),
                },
                input_schema: object_schema(),
                output_schema: object_schema(),
                graph: GraphDefinitionV1 {
                    schema_version: 1,
                    entry_node_id: "build".to_string(),
                    nodes: vec![
                        node(
                            "build",
                            GraphNodeKindV1::Tool,
                            json!({"reference": "shell"}),
                        ),
                        node(
                            "validate",
                            GraphNodeKindV1::Validator,
                            json!({"deterministic": true}),
                        ),
                        node(
                            "loop",
                            GraphNodeKindV1::Loop,
                            json!({"feedbackSchema": {"type": "object"}}),
                        ),
                        node("output", GraphNodeKindV1::Output, json!({})),
                    ],
                    edges: vec![
                        edge("build", "validate"),
                        edge("validate", "loop"),
                        edge("loop", "output"),
                        feedback,
                    ],
                },
                requested_capabilities: capabilities.clone(),
                budget: FlowBudgetV1::default(),
                risk_class: AgentRiskClassV1::Medium,
                pending_decisions: Vec::new(),
            },
            capabilities,
        )
    }

    #[test]
    fn bounded_development_loop_validates_and_compiles_into_existing_harness() {
        let (spec, capabilities) = valid_spec();
        let report = validate_flow_spec(&spec, &capabilities);
        assert!(report.valid, "{:#?}", report.issues);
        let plan = compile_flow(&spec, &capabilities).expect("compile");
        assert_eq!(plan.nodes[0].target, HarnessNodeTargetV1::ToolRegistry);
        assert_eq!(plan.nodes[1].target, HarnessNodeTargetV1::RuntimeValidator);
        assert_eq!(plan.nodes[2].target, HarnessNodeTargetV1::RuntimeLoop);
    }

    #[test]
    fn unbounded_cycle_is_rejected() {
        let (mut spec, capabilities) = valid_spec();
        spec.graph.edges.last_mut().expect("feedback").loop_policy = None;
        let report = validate_flow_spec(&spec, &capabilities);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "graph.cycle.unbounded"));
    }

    #[test]
    fn capability_requests_cannot_expand_the_execution_context() {
        let (mut spec, capabilities) = valid_spec();
        spec.requested_capabilities
            .tools
            .insert("browser".to_string());
        let report = validate_flow_spec(&spec, &capabilities);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "flow.capability.expansion"));
    }

    #[test]
    fn run_trace_source_survives_draft_normalization() {
        let (mut spec, capabilities) = valid_spec();
        let run_id = Uuid::new_v4();
        spec.source = FlowSourceV1::RunTrace {
            run_id,
            trace_hash: "trace-v1".to_string(),
        };
        let draft = FlowDraftV1::new(Uuid::new_v4(), spec, &capabilities);
        assert!(matches!(
            draft.spec.source,
            FlowSourceV1::RunTrace { run_id: value, .. } if value == run_id
        ));
    }
}
