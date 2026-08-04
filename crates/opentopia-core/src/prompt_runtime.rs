use crate::model::ExperienceMode;
use crate::model_context::{
    content_fingerprint, ContextCacheScope, ContextItemKind, ContextRole, ContextSensitivity,
    ModelContextItem,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentPersonality {
    Focused,
    Professional,
    Warm,
}

impl Default for AgentPersonality {
    fn default() -> Self {
        Self::Professional
    }
}

impl AgentPersonality {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Focused => "focused",
            Self::Professional => "professional",
            Self::Warm => "warm",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentAutonomy {
    Guided,
    Balanced,
    Proactive,
}

impl Default for AgentAutonomy {
    fn default() -> Self {
        Self::Balanced
    }
}

impl AgentAutonomy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Guided => "guided",
            Self::Balanced => "balanced",
            Self::Proactive => "proactive",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MultiAgentMode {
    Off,
    Explicit,
    Adaptive,
}

impl Default for MultiAgentMode {
    fn default() -> Self {
        Self::Explicit
    }
}

impl MultiAgentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Explicit => "explicit",
            Self::Adaptive => "adaptive",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressUpdateMode {
    Milestones,
    Balanced,
    Frequent,
}

impl Default for ProgressUpdateMode {
    fn default() -> Self {
        Self::Balanced
    }
}

impl ProgressUpdateMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Milestones => "milestones",
            Self::Balanced => "balanced",
            Self::Frequent => "frequent",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentRuntimeSettings {
    pub personality: AgentPersonality,
    pub autonomy: AgentAutonomy,
    pub multi_agent: MultiAgentMode,
    pub progress_updates: ProgressUpdateMode,
}

impl Default for AgentRuntimeSettings {
    fn default() -> Self {
        Self {
            personality: AgentPersonality::Professional,
            autonomy: AgentAutonomy::Balanced,
            multi_agent: MultiAgentMode::Explicit,
            progress_updates: ProgressUpdateMode::Balanced,
        }
    }
}

impl AgentRuntimeSettings {
    pub fn content_hash(&self) -> String {
        content_fingerprint(serde_json::to_vec(self).unwrap_or_default().as_slice())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSurface {
    Core,
    Desktop,
    Cli,
}

impl RuntimeSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Desktop => "desktop",
            Self::Cli => "cli",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PromptRuntimeCapabilities {
    pub surface: RuntimeSurface,
    pub multi_agent_available: bool,
    pub max_parallel_agents: usize,
    /// Maximum nesting depth for internal agents. `1` means the main agent may
    /// spawn children but those children cannot spawn their own.
    pub max_agent_depth: u8,
    pub request_user_input_available: bool,
}

impl Default for PromptRuntimeCapabilities {
    fn default() -> Self {
        Self {
            surface: RuntimeSurface::Core,
            multi_agent_available: false,
            max_parallel_agents: 0,
            max_agent_depth: 0,
            // Every field here describes the absence of a capability, and the
            // structured ask is only reachable in plan mode, which is never the
            // default. Defaulting this to `true` would make the clarification
            // module promise a channel the runtime rejects.
            request_user_input_available: false,
        }
    }
}

pub fn compile_runtime_prompt_modules(
    settings: &AgentRuntimeSettings,
    capabilities: PromptRuntimeCapabilities,
) -> Vec<ModelContextItem> {
    let mut modules = vec![
        prompt_module(
            "personality",
            "conditional",
            "agentRuntime.personality",
            settings.personality.as_str(),
            personality_instruction(settings.personality),
        ),
        prompt_module(
            "autonomy",
            "conditional",
            "agentRuntime.autonomy",
            settings.autonomy.as_str(),
            autonomy_instruction(settings.autonomy),
        ),
        prompt_module(
            "progress_updates",
            "conditional",
            "agentRuntime.progressUpdates",
            settings.progress_updates.as_str(),
            progress_instruction(settings.progress_updates),
        ),
        prompt_module(
            "output_contract",
            "conditional",
            "runtime.surface",
            capabilities.surface.as_str(),
            output_contract_instruction_compact(capabilities.surface),
        ),
        prompt_module(
            "clarification_policy",
            "conditional",
            "runtime.requestUserInput",
            if capabilities.request_user_input_available {
                "available"
            } else {
                "unavailable"
            },
            clarification_policy_instruction_compact(capabilities.request_user_input_available),
        ),
    ];

    if capabilities.surface == RuntimeSurface::Desktop {
        modules.push(prompt_module(
            "desktop_protocol",
            "conditional",
            "runtime.surface",
            capabilities.surface.as_str(),
            desktop_protocol_instruction_compact(),
        ));
    }

    modules.push(
        prompt_module(
            "multi_agent_policy",
            "conditional",
            "agentRuntime.multiAgent",
            settings.multi_agent.as_str(),
            multi_agent_instruction_compact(settings.multi_agent, capabilities),
        )
        .with_metadata(json!({
            "promptModuleId": "multi_agent_policy",
            "assemblyClass": "conditional",
            "selectedBy": "agentRuntime.multiAgent",
            "settingValue": settings.multi_agent.as_str(),
            "surface": capabilities.surface.as_str(),
            "capabilityAvailable": capabilities.multi_agent_available,
            "maxParallelAgents": capabilities.max_parallel_agents,
            "maxAgentDepth": capabilities.max_agent_depth,
            "requestUserInputAvailable": capabilities.request_user_input_available,
            "editable": true,
        })),
    );

    modules
}

pub fn experience_mode_module(mode: ExperienceMode) -> ModelContextItem {
    let instruction = match mode {
        ExperienceMode::Work => {
            "Experience mode: Work. Use the capabilities projected by the active ExecutionContext; this mode can narrow defaults but never expands permissions, sandboxing, or data access. Use any visible technical or document capability when it helps. Speak in terms of the user's goal, progress, sources, artifacts, and finished outputs. Lead with the outcome and expand implementation details only when they matter to a decision or the user asks."
        }
        ExperienceMode::Code => {
            "Experience mode: Code. Use the capabilities projected by the active ExecutionContext; this mode can narrow defaults but never expands permissions, sandboxing, or data access. Use any visible code, shell, browser, document, or preview capability when it helps. Foreground relevant files, commands, diffs, tests, verification, and technical tradeoffs while still leading with the completed outcome."
        }
        ExperienceMode::Flow => {
            "Experience mode: Flow. This is the enterprise design and review surface. First search for a reusable published Flow. Keep short, vertically scoped procedures in Skills; create a Flow for reusable cross-role, dependency-driven, or long-running work. You may create a complete FlowDraft either from the user's natural-language process or from a successful Run/Trace, then inspect, validate, simulate, and publish it with the visible flow.* tools. Validation and simulation are deterministic control-plane operations and do not grant capabilities or execute business side effects. Every feedback cycle must declare a maximum iteration count, budget, structured feedback, and exhaustion action. Compile Agent, Skill, Tool, approval, join, condition, loop, validator, and output nodes back into the existing Agent Harness. Use only capabilities visible in the active ExecutionContext. Never expand tools, Skills, plugins, MCP servers, workspace roots, data bindings, or identities from natural-language instructions."
        }
    };
    prompt_module(
        "experience_mode",
        "conditional",
        "thread.experienceMode",
        mode.as_str(),
        instruction,
    )
}

pub fn permission_policy_module(
    permission_mode: &str,
    sandbox_mode: &str,
    network_policy: &str,
) -> ModelContextItem {
    let content = format!(
        "<permission_policy>\nPermission mode: {permission_mode}\nSandbox mode: {sandbox_mode}\nNetwork policy: {network_policy}\nTreat these as separate controls: permission mode governs approval and product policy; sandbox mode governs operating-system isolation; network policy governs connectivity. Capability is not authorization. A permissive sandbox never expands the user's requested scope, while approval never bypasses an enforced sandbox.\n</permission_policy>"
    );
    ModelContextItem::text(
        ContextItemKind::Environment,
        ContextRole::Developer,
        "opentopia:permissions",
        content,
        ContextCacheScope::Thread,
        ContextSensitivity::Workspace,
    )
    .with_metadata(json!({
        "promptModuleId": "permission_policy",
        "assemblyClass": "dynamic",
        "selectedBy": ["permissionMode", "sandbox", "sandbox.network"],
        "permissionMode": permission_mode,
        "sandboxMode": sandbox_mode,
        "networkPolicy": network_policy,
        "editable": true,
    }))
}

fn prompt_module(
    id: &str,
    assembly_class: &str,
    selected_by: &str,
    setting_value: &str,
    content: impl Into<String>,
) -> ModelContextItem {
    let cache_scope = if assembly_class == "fixed" {
        ContextCacheScope::Stable
    } else {
        ContextCacheScope::Thread
    };
    ModelContextItem::text(
        ContextItemKind::DeveloperInstructions,
        ContextRole::Developer,
        format!("opentopia:prompt:{id}"),
        content,
        cache_scope,
        ContextSensitivity::Public,
    )
    .with_metadata(json!({
        "promptModuleId": id,
        "assemblyClass": assembly_class,
        "selectedBy": selected_by,
        "settingValue": setting_value,
        "editable": assembly_class != "fixed",
    }))
}

fn personality_instruction(personality: AgentPersonality) -> &'static str {
    match personality {
        AgentPersonality::Focused => {
            "Communication personality: Focused. Be direct, compact, and work-oriented. Avoid social filler and decorative narration. Match the user's technical level, lead with the result, and use structure only when it makes the result easier to scan."
        }
        AgentPersonality::Professional => {
            "Communication personality: Professional. Be calm, candid, and collaborative. Match the user's technical level, explain consequential reasoning and tradeoffs with concrete evidence, and keep routine details concise. Lead with the result and make the next decision easy to evaluate."
        }
        AgentPersonality::Warm => {
            "Communication personality: Warm. Sound natural, attentive, and approachable while remaining precise. Anticipate common questions, guide unfamiliar users without talking down to them, and preserve independent judgment. Lead with the result and avoid performative enthusiasm or empty reassurance."
        }
    }
}

fn autonomy_instruction(autonomy: AgentAutonomy) -> &'static str {
    match autonomy {
        AgentAutonomy::Guided => {
            "Autonomy policy: Guided. Inspect and diagnose actively, but pause before consequential design choices, broad refactors, new dependencies, external writes, or other actions whose outcome the user would reasonably want to choose. Continue without asking for routine, reversible implementation details inside an already approved scope."
        }
        AgentAutonomy::Balanced => {
            "Autonomy policy: Balanced. Carry approved change and build requests through implementation and proportionate verification. Make conservative, reversible assumptions inside scope. Ask only when a missing choice would materially change architecture, product behavior, risk, cost, or authority."
        }
        AgentAutonomy::Proactive => {
            "Autonomy policy: Proactive. Drive approved work to a finished, verified outcome. Resolve routine ambiguity from repository evidence, choose sensible reversible defaults, and complete normal follow-up steps inside scope without waiting for permission. Stop only for a real authority boundary, a high-impact product choice with no reliable evidence, or an external blocker."
        }
    }
}

fn progress_instruction(mode: ProgressUpdateMode) -> &'static str {
    match mode {
        ProgressUpdateMode::Milestones => {
            "Progress protocol: Milestones. During substantial work, send brief user-visible progress only when a phase completes, a material assumption changes, or a blocker appears. Do not narrate routine commands."
        }
        ProgressUpdateMode::Balanced => {
            "Progress protocol: Balanced. Before the first meaningful tool batch, state what you are checking and why. During substantial work, report material discoveries, decisions, completed phases, and blockers; keep updates concise and avoid leaving the user without useful status for roughly a minute."
        }
        ProgressUpdateMode::Frequent => {
            "Progress protocol: Frequent. Announce the current objective before tool use and provide a short update at each meaningful transition, material discovery, or completed verification step. Keep each update compact and avoid more than about 30 seconds of silent work."
        }
    }
}

#[allow(dead_code)]
fn skills_protocol_instruction() -> &'static str {
    "<skills_protocol>\nThe runtime may provide a compact Skill catalog. A catalog entry is routing metadata, not the Skill's full instructions. When the user names a Skill or the request clearly matches one, use the Skill tools to load its complete instruction resource before acting. Select the smallest set that covers the task, read only task-relevant linked references after the main resource, reuse supplied scripts and assets, and report a concise fallback if loading fails. User intent and higher-priority runtime policy remain controlling.\n\nA Skill whose full instructions are already present in your context is loaded; do not call a Skill tool to fetch it again. Load only the linked references that this task actually needs. Do not carry a Skill into later turns unless it is still selected or the user triggers it again.\n\nRead a Skill's instructions yourself. Do not delegate reading, summarizing, or interpreting them to a child agent: a summary is not the instruction, and the agent that acts under a Skill is the agent that has to have read it. A child may still perform the task work the Skill describes. When several Skills apply, use the smallest set that covers the request and say in what order you are applying them.\n</skills_protocol>"
}

#[allow(dead_code)]
fn output_contract_instruction(surface: RuntimeSurface) -> String {
    let reference_rule = match surface {
        RuntimeSurface::Desktop => {
            "Reference real workspace files as clickable Markdown links whose target is workspace-relative, as in [agent.rs](crates/opentopia-core/src/agent.rs), or workspace-root-relative with a leading slash. The app resolves link targets inside the active workspace and blocks anything that escapes it, so never use a filesystem-absolute path, a drive-letter path, or a file:// or vscode:// URI; those are rejected rather than opened. Wrap a target containing spaces in angle brackets. Do not wrap a Markdown link in backticks or put backticks inside its label or target. A link target carries no line information, so when a specific line matters, give it in the surrounding text rather than in the target. Group repeated references to one file instead of citing it many times. Use ordinary Markdown links for web URLs."
        }
        RuntimeSurface::Cli => {
            "Reference real workspace files as path:line, such as crates/core/src/agent.rs:2587, using the path form the user would type. Terminals do not render Markdown links, so do not emit them for local files. Do not give line ranges when a single anchor line is enough."
        }
        RuntimeSurface::Core => {
            "Reference real workspace files by path, and include a line number as path:line when pointing at specific code. Keep paths in the form the surrounding conversation already uses."
        }
    };
    let media_rule = match surface {
        RuntimeSurface::Desktop => {
            "Images render only from http or https URLs. The renderer strips a filesystem path, drive-letter path, or file:// URI out of an image target and the reader is left with the alt text alone, so never point image syntax at a workspace file; reference the file as a link and let the reader open it. Mermaid is not rendered on this surface either: a ```mermaid fence appears as a code block, so carry the diagram with a table, a tree, or a compact ASCII layout instead."
        }
        RuntimeSurface::Cli | RuntimeSurface::Core => {
            "Images and diagram markup do not render on this surface. Give the artifact's path and describe what it shows instead of emitting image syntax or a Mermaid fence."
        }
    };
    let markup_rule = match surface {
        RuntimeSurface::Desktop => {
            "Your response is rendered as GitHub-flavored Markdown. Follow CommonMark structure: leave a blank line before any list and between a heading and the content that follows it, or the output will not render correctly."
        }
        RuntimeSurface::Cli | RuntimeSurface::Core => {
            "Your response is shown as text with limited or no Markdown rendering. Prefer short paragraphs and plain punctuation over heavy markup, and never rely on a table or nested formatting to carry meaning."
        }
    };
    format!(
        "<output_contract>\n{markup_rule}\n\nDo not over-format. Use bold, headings, lists, and tables only where they make the answer easier to read than prose would, and prefer the smallest structure that does the job. A short answer usually needs no structure at all.\n\n{reference_rule}\n\n{media_rule}\n\nUse a visualization only when it makes an important relationship materially clearer than prose or a short list would. Reach for one when you are comparing several precise mappings or repeated fields, when one source or decision fans out to three or more downstream consumers, when three or more interdependent steps or state transitions are involved, or when the subject is a hierarchy, ownership graph, or layout that reads poorly in linear text. Pick the smallest form that works: a table for mappings and comparisons, a flow or timeline for sequence and change, a tree for hierarchy, a wireframe for layout. Do not add one merely because an answer has several parts. A large ASCII diagram counts as a visualization; compact notation and small inline examples do not.\n\nNever fabricate a rendered artifact. Markdown alone does not change application state, create a file, or complete an action; only a real tool result does.\n</output_contract>"
    )
}

#[allow(dead_code)]
fn clarification_policy_instruction(request_user_input_available: bool) -> &'static str {
    if request_user_input_available {
        "<clarification_policy>\nThe structured `request_user_input` tool is available this turn. Strongly prefer making a reasonable, reversible assumption and carrying the request forward over stopping to ask. Ask only when the answer cannot be determined from the workspace and a wrong assumption would materially change architecture, product behavior, scope, risk, cost, or authority. When you do ask, use `request_user_input` with one to three concise questions and concrete trade-offs for each option, then continue once the answers return. Do not ask about routine implementation details you can decide and later revise.\n</clarification_policy>"
    } else {
        "<clarification_policy>\nThe structured `request_user_input` tool is not available this turn, so you cannot present the user a selectable set of options. Strongly prefer making a reasonable, reversible assumption and carrying the request forward. If you genuinely cannot proceed without a decision the user alone can make, end the turn with a short plain-text question in your final response and state the assumption you would otherwise make. Never render a multiple-choice prompt as ordinary assistant text or imply the user can select an option; nothing will capture that selection.\n</clarification_policy>"
    }
}

#[allow(dead_code)]
fn desktop_protocol_instruction() -> &'static str {
    "<desktop_protocol>\nYou are running inside the OpenTopia desktop workbench. Use workspace-relative paths when identifying project files and rely on typed artifacts, previews, event records, approvals, and tool results as the source of UI truth. Do not emit Codex-specific `::directive` tokens or pretend that Markdown alone changed application state. Create or open previews through available tools and report only states observed from OpenTopia. The activity timeline separates logical model context, provider transport, tool execution, approvals, and final output; keep those distinctions accurate.\n</desktop_protocol>"
}

#[allow(dead_code)]
fn multi_agent_instruction(
    mode: MultiAgentMode,
    capabilities: PromptRuntimeCapabilities,
) -> String {
    if !capabilities.multi_agent_available || mode == MultiAgentMode::Off {
        return "<multi_agent_policy>\nInternal multi-agent delegation is disabled for this runtime. Complete the task in the current agent and do not claim to have spawned or contacted child agents.\n</multi_agent_policy>".to_string();
    }

    let capacity = capabilities.max_parallel_agents.max(1);
    let activation = match mode {
        MultiAgentMode::Explicit => {
            "Use internal agents only when the user explicitly requests delegation or when an applicable repository or Skill instruction explicitly requires it. Mere availability is not authorization to delegate."
        }
        MultiAgentMode::Adaptive => {
            "You may delegate when a concrete, bounded subtask can run independently alongside useful local work and the expected latency or context-isolation benefit exceeds coordination cost. Do not delegate trivial, tightly coupled, or sequential work."
        }
        MultiAgentMode::Off => unreachable!(),
    };
    let depth_rule = match capabilities.max_agent_depth {
        0 | 1 => {
            "Only you may spawn agents; a child cannot spawn its own children. Do not design a plan that depends on a child delegating further, and do not instruct a child to spawn agents.".to_string()
        }
        depth => format!(
            "Internal agents may nest up to {depth} levels below you. A child at the deepest level cannot spawn further children, so do not design a plan that depends on it delegating."
        ),
    };
    format!(
        "<multi_agent_policy>\nInternal agent tools are available with up to {capacity} active child tasks per parent. {activation} {depth_rule}\n\nEvery child inherits the parent permission and sandbox boundary. Its assigned workspace may be shared or an isolated worktree; the workspace root and execution contract supplied by the runtime are authoritative. Isolation never expands authorization.\n\nControl how much of your conversation a child starts with using `fork_turns`: `none` gives it only the task message you write, a positive integer copies that many of the most recent turns, and `all` copies the full history. Prefer `none` with a self-contained task description; it keeps the child's context small and its result easier to trust. Copy history only when the task genuinely depends on earlier discussion, and remember that a large fork costs tokens on every round the child runs.\n\nA child runs on the same model and reasoning effort as you unless its `agent_type` profile overrides them. Choose a non-default profile only when the user, an applicable repository instruction, or a Skill calls for that specialization, or when the subtask is clearly cheaper or harder than the main line of work. Do not pair a large history fork with a lighter profile: a child given full context is expected to reason at the parent's level.\n\nGive each child a disjoint scope and enough context to act without guessing. Use shared read-only work for investigation; sequence overlapping writes in a shared workspace; use isolated worktrees for genuinely independent implementations when the runtime offers them. Children return evidence or an integration-ready deliverable to you. You own the user-facing result: select and semantically integrate child work, while the harness performs mechanical worktree operations and reports conflicts. Review evidence before relying on it; a terminal status is not proof of success. Do not finish while required child work or unread results remain.\n</multi_agent_policy>"
    )
}

// The legacy protocol builders above remain available for compatibility with
// callers that inspect them in older integrations. Runtime assembly uses the
// compact variants below so the stable base prompt is not repeated by every
// developer module.
#[allow(dead_code)]
fn skills_protocol_instruction_compact() -> &'static str {
    "<skills_protocol>\nThe Skill catalog is routing metadata, not full instructions. When a Skill is named or clearly matches the request, load its complete resource before acting; choose the smallest applicable set and read only needed references. A loaded Skill needs no second fetch and is not carried into later turns unless selected again. Read the instructions yourself; a child may do the work but may not replace that reading with a summary.\n</skills_protocol>"
}

fn output_contract_instruction_compact(surface: RuntimeSurface) -> String {
    let (format_rule, reference_rule, media_rule) = match surface {
        RuntimeSurface::Desktop => (
            "Responses render as GitHub-flavored Markdown; leave a blank line before lists and after headings.",
            "Use workspace-relative Markdown links for local files; never use absolute, drive-letter, file://, or vscode:// targets.",
            "Images render only from http(s) URLs; reference local images as links. Mermaid is shown as code, so use a table or compact text instead.",
        ),
        RuntimeSurface::Cli => (
            "Responses have limited Markdown rendering; prefer concise paragraphs and plain punctuation.",
            "Use typed workspace paths and path:line for code references; do not emit local Markdown links.",
            "Do not emit image syntax or Mermaid; give the artifact path and describe it.",
        ),
        RuntimeSurface::Core => (
            "Prefer concise structure and do not rely on rich Markdown rendering.",
            "Use workspace paths and path:line for specific code references.",
            "Describe non-rendered images or diagrams by artifact path.",
        ),
    };
    format!(
        "<output_contract>\n{format_rule}\n{reference_rule}\n{media_rule}\nUse the smallest structure that improves clarity. Only observed tool results prove that an artifact or state change exists.\n</output_contract>"
    )
}

fn clarification_policy_instruction_compact(request_user_input_available: bool) -> &'static str {
    if request_user_input_available {
        "<clarification_policy>\nThe structured `request_user_input` tool is available. Prefer a reversible assumption; ask only when workspace evidence cannot determine a choice whose wrong value would materially change behavior, scope, risk, cost, or authority.\n</clarification_policy>"
    } else {
        "<clarification_policy>\nNo structured input tool is available. Prefer a reversible assumption; if the user alone must decide a material issue, ask one short plain-text question in the final response. Never present an ordinary-text multiple-choice prompt.\n</clarification_policy>"
    }
}

fn desktop_protocol_instruction_compact() -> &'static str {
    "<desktop_protocol>\nUse workspace-relative paths and treat typed artifacts, previews, events, approvals, and tool results as UI truth. Do not emit Codex `::directive` tokens or claim Markdown changed state. Keep model context, provider transport, tool execution, approvals, and final output distinct.\n</desktop_protocol>"
}

fn multi_agent_instruction_compact(
    mode: MultiAgentMode,
    capabilities: PromptRuntimeCapabilities,
) -> String {
    if !capabilities.multi_agent_available || mode == MultiAgentMode::Off {
        return "<multi_agent_policy>\nInternal delegation is disabled. Complete the task in the current agent and do not claim to have spawned children.\n</multi_agent_policy>".to_string();
    }
    let capacity = capabilities.max_parallel_agents.max(1);
    let activation = match mode {
        MultiAgentMode::Explicit => "Delegate only when the user or an applicable repository/Skill instruction explicitly requires it.",
        MultiAgentMode::Adaptive => "Delegate only bounded work that benefits from independent execution or context isolation.",
        MultiAgentMode::Off => unreachable!(),
    };
    let depth = if capabilities.max_agent_depth <= 1 {
        "Children cannot spawn grandchildren."
    } else {
        "Respect the configured child depth."
    };
    format!(
        "<multi_agent_policy>\nUp to {capacity} child tasks are available. {activation} {depth} Children inherit permissions and sandbox boundaries; the runtime may assign a shared workspace or isolated worktree, which never expands authorization. Prefer `fork_turns: none` with a self-contained task and disjoint scope. Sequence overlapping shared writes; use isolated worktrees only for independent changes. Children return evidence or integration-ready deliverables; you remain responsible for selecting, semantically integrating, and verifying the unified user result while the harness handles mechanical worktree operations. Do not finish while required child work or unread results remain.\n</multi_agent_policy>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_hash_changes_with_policy() {
        let settings = AgentRuntimeSettings::default();
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json["personality"], "professional");
        assert_eq!(json["multiAgent"], "explicit");

        let mut changed = settings.clone();
        changed.multi_agent = MultiAgentMode::Adaptive;
        assert_ne!(settings.content_hash(), changed.content_hash());
        assert_eq!(
            serde_json::from_value::<AgentRuntimeSettings>(json).unwrap(),
            settings
        );
    }

    #[test]
    fn compiled_modules_expose_assembly_metadata() {
        let modules = compile_runtime_prompt_modules(
            &AgentRuntimeSettings::default(),
            PromptRuntimeCapabilities {
                surface: RuntimeSurface::Desktop,
                multi_agent_available: true,
                max_parallel_agents: 6,
                max_agent_depth: 1,
                request_user_input_available: true,
            },
        );

        let desktop = modules
            .iter()
            .find(|item| item.metadata["promptModuleId"] == "desktop_protocol")
            .expect("desktop module");
        assert_eq!(desktop.metadata["assemblyClass"], "conditional");
        assert!(desktop.text_content().contains("typed artifacts"));

        let multi_agent = modules
            .iter()
            .find(|item| item.metadata["promptModuleId"] == "multi_agent_policy")
            .expect("multi-agent module");
        assert_eq!(multi_agent.metadata["maxParallelAgents"], 6);
        assert_eq!(multi_agent.metadata["maxAgentDepth"], 1);
        assert!(multi_agent
            .text_content()
            .contains("explicitly requires it"));
        assert!(multi_agent
            .text_content()
            .contains("Children cannot spawn grandchildren"));
        assert!(multi_agent.text_content().contains("fork_turns: none"));
    }

    #[test]
    fn output_contract_and_clarification_modules_track_the_surface_and_tool() {
        let desktop = compile_runtime_prompt_modules(
            &AgentRuntimeSettings::default(),
            PromptRuntimeCapabilities {
                surface: RuntimeSurface::Desktop,
                multi_agent_available: false,
                max_parallel_agents: 0,
                max_agent_depth: 0,
                request_user_input_available: true,
            },
        );
        let output = desktop
            .iter()
            .find(|item| item.metadata["promptModuleId"] == "output_contract")
            .expect("output contract module");
        assert_eq!(output.metadata["assemblyClass"], "conditional");
        assert_eq!(output.metadata["settingValue"], "desktop");
        assert!(output.text_content().contains("workspace-relative"));
        assert!(output.text_content().contains("never use absolute"));
        // The renderer only accepts http(s) image targets and has no mermaid
        // plugin, so both rules have to say what actually happens instead of
        // copying a contract written for another renderer.
        assert!(output
            .text_content()
            .contains("Images render only from http(s) URLs"));
        assert!(output.text_content().contains("Mermaid is shown as code"));

        assert!(!desktop
            .iter()
            .any(|item| item.metadata["promptModuleId"] == "skills_protocol"));

        let clarification = desktop
            .iter()
            .find(|item| item.metadata["promptModuleId"] == "clarification_policy")
            .expect("clarification module");
        assert_eq!(clarification.metadata["settingValue"], "available");
        assert!(clarification.text_content().contains("request_user_input"));

        let cli = compile_runtime_prompt_modules(
            &AgentRuntimeSettings::default(),
            PromptRuntimeCapabilities {
                surface: RuntimeSurface::Cli,
                multi_agent_available: false,
                max_parallel_agents: 0,
                max_agent_depth: 0,
                request_user_input_available: false,
            },
        );
        let cli_output = cli
            .iter()
            .find(|item| item.metadata["promptModuleId"] == "output_contract")
            .expect("output contract module");
        assert!(cli_output.text_content().contains("path:line"));
        assert!(!cli_output
            .text_content()
            .contains("clickable Markdown links"));
        assert!(cli_output
            .text_content()
            .contains("limited Markdown rendering"));
        assert!(cli_output
            .text_content()
            .contains("Do not emit image syntax or Mermaid"));
        assert!(!cli_output.text_content().contains("http or https URLs"));

        let cli_clarification = cli
            .iter()
            .find(|item| item.metadata["promptModuleId"] == "clarification_policy")
            .expect("clarification module");
        assert_eq!(cli_clarification.metadata["settingValue"], "unavailable");
        assert!(cli_clarification
            .text_content()
            .contains("Never present an ordinary-text multiple-choice prompt"));
    }

    #[test]
    fn disabled_multi_agent_policy_is_explicit_even_when_capability_exists() {
        let mut settings = AgentRuntimeSettings::default();
        settings.multi_agent = MultiAgentMode::Off;
        let modules = compile_runtime_prompt_modules(
            &settings,
            PromptRuntimeCapabilities {
                surface: RuntimeSurface::Core,
                multi_agent_available: true,
                max_parallel_agents: 6,
                max_agent_depth: 1,
                request_user_input_available: false,
            },
        );
        let multi_agent = modules.last().expect("multi-agent module");
        assert!(multi_agent
            .text_content()
            .contains("delegation is disabled"));
        assert_eq!(multi_agent.metadata["capabilityAvailable"], true);
    }

    #[test]
    fn permission_module_separates_policy_from_isolation() {
        let item = permission_policy_module("approve", "workspace-write", "deny");
        let content = item.text_content();
        assert!(content.contains("Capability is not authorization"));
        assert!(content.contains("approval never bypasses an enforced sandbox"));
        assert_eq!(item.metadata["assemblyClass"], "dynamic");
    }
}
