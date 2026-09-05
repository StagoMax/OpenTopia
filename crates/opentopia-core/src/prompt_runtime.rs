use crate::model::ExperienceMode;
use crate::model_context::{
    content_fingerprint, ContextCacheScope, ContextItemKind, ContextRole, ContextSensitivity,
    ModelContextItem,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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
            // Every field here describes the absence of a capability.
            // Structured decisions are model-driven in every root mode, but a
            // runtime that did not install the tool must not promise the channel.
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
            "Experience mode: Work. Prefer user-goal, source, artifact, and deliverable language. Use visible technical or document capabilities when they help. This mode can narrow defaults but never expands authorization, permissions, sandboxing, or data access."
        }
        ExperienceMode::Code => {
            "Experience mode: Code. Foreground relevant files, commands, diffs, tests, verification, and technical tradeoffs. Use visible code, shell, browser, document, or preview capabilities when they help. This mode can narrow defaults but never expands authorization, permissions, sandboxing, or data access."
        }
        ExperienceMode::Flow => {
            "Experience mode: Flow. This is the enterprise design, run, and review surface. It inherits the visible code, shell, browser, document, preview, plugin, and MCP capabilities available to Work and Code, and adds Agent and Flow control-plane capabilities. Prefer one capable Agent for most vertical tasks. Search existing Agents with agent_search before using agent_create to create a draft Agent configuration from natural language; do not expose the internal template abstraction to the user and do not publish automatically. Use a Flow only when event-driven ordering, branching, joins, long-running control, or explicit human gates are genuinely required. A Flow Agent node owns a Trigger expression and a reference to an independently reusable Agent. Agent Final subscriptions are represented as Trigger sources; do not invent a second Final event system. Preserve the raw ingress payload as @Flow.input and treat @Trigger.input as the current node activation payload. Connection tools remain callable capabilities and may fetch records from identifiers in arbitrary source-native Trigger parameters. You may create a complete FlowDraft either from the user's natural-language process or from a successful Run/Trace, then inspect, validate, simulate, and publish it with the visible flow_* tools. Validation and simulation are deterministic control-plane operations and do not grant capabilities or execute business side effects. Run only an immutable published version with flow_run; use flow_status to inspect its durable NodeRun trace, and pause, resume, approve, or cancel only at explicit runtime boundaries. Every feedback cycle must declare a maximum iteration count, budget, structured feedback, and exhaustion action. Compile Agent, Skill, Tool, approval, join, condition, loop, validator, and output nodes back into the existing Agent Harness. Use only capabilities visible in the active ExecutionContext. Never expand tools, Skills, plugins, MCP servers, workspace roots, data bindings, or identities from natural-language instructions."
        }
    };
    let runtime_mode = match mode {
        ExperienceMode::Work => "work_mode",
        ExperienceMode::Code => "code",
        ExperienceMode::Flow => "flow",
    };
    let service_name = mode.codex_service_name();
    let runtime_context = match service_name {
        Some(service_name) => format!(
            "<runtime_context>\nsurface = codex_desktop\nmode = {runtime_mode}\nservice_name = {service_name}\n</runtime_context>"
        ),
        None => format!(
            "<runtime_context>\nsurface = codex_desktop\nmode = {runtime_mode}\n</runtime_context>"
        ),
    };
    let mut module = prompt_module(
        "experience_mode",
        "conditional",
        "thread.experienceMode",
        mode.as_str(),
        format!("{runtime_context}\n{instruction}"),
    );
    if let Some(metadata) = module.metadata.as_object_mut() {
        metadata.insert("surface".to_string(), json!("codex_desktop"));
        metadata.insert("runtimeMode".to_string(), json!(runtime_mode));
        if let Some(service_name) = service_name {
            metadata.insert("serviceName".to_string(), json!(service_name));
        }
    }
    module
}

pub fn permission_policy_module(
    permission_mode: &str,
    sandbox_mode: &str,
    network_policy: &str,
) -> ModelContextItem {
    let permission_rule = match permission_mode {
        "chat" => "Tool execution and workspace mutation are disabled; answer from available context and read-only evidence already supplied by the runtime.",
        "read_only" => "Use only non-mutating inspection. Do not attempt writes or side effects, and do not ask for approval for an operation this mode denies.",
        "auto" => "The policy engine may route approval-required actions to automatic review. Submit only actions already authorized by the user's request; a reviewer decision does not broaden scope.",
        "approve" => "When policy requires approval, request it through the runtime and wait for the decision. Do not work around a denial or rejection.",
        "full_access" => "Host access is broad, but destructive actions may still require approval. Broad access does not authorize unrelated mutation or external effects.",
        "unrestricted" => "Approval prompts are disabled and policy asks may be allowed automatically. Exercise the same scope and destructive-action restraint because lack of an approval prompt is not user authorization.",
        _ => "Follow the runtime's approval decision and never treat tool availability as authorization.",
    };
    let sandbox_rule = match sandbox_mode {
        "read-only" => "The operating-system sandbox prohibits filesystem writes.",
        "workspace-write" => "Filesystem writes are limited to configured writable workspace roots; access outside them may be denied or require a different runtime boundary.",
        "danger-full-access" => "The operating-system sandbox does not meaningfully restrict filesystem access; resolve exact targets and stay within the requested scope.",
        _ => "Treat the reported sandbox boundary as authoritative.",
    };
    let network_rule = match network_policy {
        "deny" => "Network access is unavailable; do not claim to have fetched remote data.",
        "allow" => {
            "Network access is available when the request and tool policy authorize its use."
        }
        "inherit" => {
            "Network access inherits the surrounding runtime policy and may still be denied."
        }
        _ => "Treat the reported network boundary as authoritative.",
    };
    let content = format!(
        "<permission_policy>\nPermission mode: {permission_mode}. {permission_rule}\nSandbox mode: {sandbox_mode}. {sandbox_rule}\nNetwork policy: {network_policy}. {network_rule}\nThese are separate controls: permission mode governs approval and product policy; sandbox mode governs operating-system isolation; network policy governs connectivity. Capability is not authorization. A permissive sandbox never expands the user's request, and approval never bypasses an enforced sandbox.\n</permission_policy>"
    );
    ModelContextItem::text(
        ContextItemKind::Environment,
        ContextRole::Developer,
        "opentopia:permissions",
        content,
        ContextCacheScope::Turn,
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
        // Editable and conditional modules are an epoch/tail concern. Keeping
        // them out of the reusable prefix prevents a settings or mode change
        // from rewriting previously cacheable bytes.
        ContextCacheScope::Turn
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
            "Personality override: Focused. Be especially direct, compact, and work-oriented; minimize social filler and decorative narration."
        }
        AgentPersonality::Professional => {
            "Personality override: Professional. Be calm, candid, and businesslike while retaining a collaborative voice."
        }
        AgentPersonality::Warm => {
            "Personality override: Warm. Sound especially natural, attentive, and approachable while remaining precise and preserving independent judgment."
        }
    }
}

fn autonomy_instruction(autonomy: AgentAutonomy) -> &'static str {
    match autonomy {
        AgentAutonomy::Guided => {
            "Autonomy override: Guided. Within work the user authorized, pause before consequential design choices, broad refactors, new dependencies, or external writes. Continue through routine reversible details. This setting never converts an answer, diagnosis, review, status, or plan request into permission to implement."
        }
        AgentAutonomy::Balanced => {
            "Autonomy override: Balanced. Within work the user authorized, make conservative reversible assumptions and ask only when a missing choice materially changes behavior, scope, risk, cost, or authority. This setting never changes the request type or expands authorization."
        }
        AgentAutonomy::Proactive => {
            "Autonomy override: Proactive. Drive authorized change work to a verified outcome, resolve routine ambiguity from evidence, and complete normal in-scope follow-up steps. Stop at authority boundaries or material choices without reliable evidence. This setting never converts an answer, diagnosis, review, status, or plan request into permission to implement."
        }
    }
}

fn progress_instruction(mode: ProgressUpdateMode) -> &'static str {
    match mode {
        ProgressUpdateMode::Milestones => {
            "Progress cadence override: Milestones. During substantial work, update only when a phase completes, a material assumption changes, or a blocker appears."
        }
        ProgressUpdateMode::Balanced => {
            "Progress cadence override: Balanced. Use the base commentary cadence and report material discoveries, decisions, completed phases, and blockers."
        }
        ProgressUpdateMode::Frequent => {
            "Progress cadence override: Frequent. Add a compact update at each meaningful transition, material discovery, or completed verification step."
        }
    }
}

fn output_contract_instruction_compact(surface: RuntimeSurface) -> String {
    let (format_rule, reference_rule, media_rule) = match surface {
        RuntimeSurface::Desktop => (
            "Responses render as GitHub-flavored Markdown; leave a blank line before lists and after headings.",
            "Use workspace-relative Markdown links for local files; never use absolute, drive-letter, file://, or vscode:// targets.",
            "Images render only from http(s) URLs; reference local images as links. Mermaid fences render as diagrams whose source can be copied.",
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
        // These rules describe the desktop renderer's actual media support.
        assert!(output
            .text_content()
            .contains("Images render only from http(s) URLs"));
        assert!(output
            .text_content()
            .contains("Mermaid fences render as diagrams"));

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

    #[test]
    fn work_mode_exposes_the_codex_runtime_identity_without_changing_other_modes() {
        let work = experience_mode_module(ExperienceMode::Work);
        assert!(work.text_content().contains("surface = codex_desktop"));
        assert!(work.text_content().contains("mode = work_mode"));
        assert!(work
            .text_content()
            .contains("service_name = codex_work_desktop"));
        assert_eq!(work.metadata["serviceName"], "codex_work_desktop");

        for mode in [ExperienceMode::Code, ExperienceMode::Flow] {
            let module = experience_mode_module(mode);
            assert!(module.text_content().contains("surface = codex_desktop"));
            assert!(module.metadata.get("serviceName").is_none());
            assert!(!module.text_content().contains("codex_work_desktop"));
        }
    }
}
