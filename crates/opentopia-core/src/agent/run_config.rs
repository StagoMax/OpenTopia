use super::{AgentContinuation, AgentCore, AgentTurnInput};
use crate::agent_composition::AgentProviderBinding;
use crate::agent_profiles::AgentProfile;
use crate::collaboration::AgentPath;
use crate::execution_authority::ExecutionAuthority;
use crate::flow_runtime::{
    FlowNodeExecutionOutcomeV1, FlowNodeExecutionRequestV1, FlowNodeHarness,
    FlowNodeResumeRequestV1,
};
use crate::model::{CollaborationMode, ExperienceMode, GoalRecord, ThreadModelSelection};
use crate::model_context::CompiledModelContext;
use crate::settings::AppSettings;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRunIdentity {
    turn_id: Uuid,
    invocation_id: u64,
    path: AgentPath,
}

impl AgentRunIdentity {
    pub fn root(turn_id: Uuid, invocation_id: u64) -> Self {
        Self {
            turn_id,
            invocation_id,
            path: AgentPath::root(),
        }
    }

    pub fn new(turn_id: Uuid, invocation_id: u64, path: AgentPath) -> Self {
        Self {
            turn_id,
            invocation_id,
            path,
        }
    }

    pub fn turn_id(&self) -> Uuid {
        self.turn_id
    }

    pub fn invocation_id(&self) -> u64 {
        self.invocation_id
    }

    pub fn path(&self) -> &AgentPath {
        &self.path
    }
}

/// Complete, immutable input required to prepare one executable Agent run.
pub struct AgentRunConfig {
    pub(crate) provider: Option<AgentProviderBinding>,
    pub(crate) authority: ExecutionAuthority,
    pub(crate) identity: AgentRunIdentity,
    pub(crate) experience_mode: ExperienceMode,
    pub(crate) collaboration_mode: CollaborationMode,
    pub(crate) goal: Option<GoalRecord>,
    pub(crate) profile: Option<AgentProfile>,
}

impl AgentRunConfig {
    pub fn from_settings(
        settings: &AppSettings,
        selection: Option<&ThreadModelSelection>,
        authority: ExecutionAuthority,
        identity: AgentRunIdentity,
    ) -> Self {
        Self {
            provider: Some(AgentProviderBinding::from_settings(settings, selection)),
            authority,
            identity,
            experience_mode: ExperienceMode::Code,
            collaboration_mode: CollaborationMode::Default,
            goal: None,
            profile: None,
        }
    }

    /// Keeps the provider already installed in a custom/test Agent kernel.
    pub fn using_current_provider(
        authority: ExecutionAuthority,
        identity: AgentRunIdentity,
    ) -> Self {
        Self {
            provider: None,
            authority,
            identity,
            experience_mode: ExperienceMode::Code,
            collaboration_mode: CollaborationMode::Default,
            goal: None,
            profile: None,
        }
    }

    pub fn with_experience_mode(mut self, mode: ExperienceMode) -> Self {
        self.experience_mode = mode;
        self
    }

    pub fn with_collaboration_mode(
        mut self,
        mode: CollaborationMode,
        goal: Option<GoalRecord>,
    ) -> Self {
        self.collaboration_mode = mode;
        self.goal = goal;
        self
    }

    pub fn with_profile(mut self, profile: AgentProfile) -> Self {
        self.profile = Some(profile);
        self
    }
}

/// Non-executable composition phase. Product adapters may attach runtime
/// services and activated tools, then must finalize before invoking a turn.
pub struct AgentRunDraft {
    agent: AgentCore,
    authority: ExecutionAuthority,
    identity: AgentRunIdentity,
}

impl AgentRunDraft {
    pub fn finalize(mut self) -> anyhow::Result<PreparedAgentRun> {
        if self.agent.experience_mode == ExperienceMode::Flow
            && self.agent.flow_harness_override.is_none()
        {
            let harness = AgentRunDraft {
                agent: self.agent.clone(),
                authority: self.authority.clone(),
                identity: self.identity.clone(),
            }
            .finalize_without_flow_harness()?;
            self.agent.set_flow_node_harness(Arc::new(harness));
        }
        self.finalize_without_flow_harness()
    }

    fn finalize_without_flow_harness(self) -> anyhow::Result<PreparedAgentRun> {
        self.authority
            .validate_workspace(self.authority.workspace_root())?;
        anyhow::ensure!(
            self.agent.capability_projection == *self.authority.capability_projection(),
            "prepared Agent capability projection drifted from its execution authority"
        );
        anyhow::ensure!(
            self.agent.tool_host.sandbox_config == *self.authority.sandbox_config(),
            "prepared Agent sandbox drifted from its execution authority"
        );
        anyhow::ensure!(
            self.agent.execution_authority.as_ref() == Some(&self.authority),
            "prepared Agent lost its execution authority"
        );
        anyhow::ensure!(
            self.agent.agent_turn_id == Some(self.identity.turn_id)
                && self.agent.invocation_id == self.identity.invocation_id
                && self.agent.agent_path == self.identity.path.as_str(),
            "prepared Agent invocation identity is inconsistent"
        );
        anyhow::ensure!(
            self.agent
                .enabled_bundled_plugins
                .iter()
                .all(|plugin| self.agent.capability_projection.allows_plugin(plugin)),
            "bundled plugins remain active outside the execution authority"
        );
        anyhow::ensure!(
            self.agent.tool_host.active_mcp_tools.iter().all(|tool| self
                .agent
                .capability_projection
                .allows_mcp_server(&tool.server_id.to_string())),
            "MCP tools remain active outside the execution authority"
        );
        Ok(PreparedAgentRun {
            agent: self.agent,
            authority: self.authority,
            identity: self.identity,
        })
    }
}

impl Deref for AgentRunDraft {
    type Target = AgentCore;

    fn deref(&self) -> &Self::Target {
        &self.agent
    }
}

impl DerefMut for AgentRunDraft {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.agent
    }
}

/// A run that passed composition and authority validation.
pub struct PreparedAgentRun {
    agent: AgentCore,
    authority: ExecutionAuthority,
    identity: AgentRunIdentity,
}

impl PreparedAgentRun {
    pub fn authority(&self) -> &ExecutionAuthority {
        &self.authority
    }

    pub fn identity(&self) -> &AgentRunIdentity {
        &self.identity
    }

    pub fn prepare_turn(
        &self,
        input: AgentTurnInput,
        model_context: Option<CompiledModelContext>,
    ) -> anyhow::Result<TurnExecutionContext> {
        self.authority.validate_workspace(&input.workspace_root)?;
        anyhow::ensure!(
            input.permission_mode == self.authority.permission_mode(),
            "turn permission mode does not match its execution authority"
        );
        Ok(TurnExecutionContext {
            input,
            model_context,
        })
    }

    pub fn validate_continuation(&self, continuation: &AgentContinuation) -> anyhow::Result<()> {
        let authority = continuation
            .execution_authority
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("continuation is missing its execution authority"))?;
        anyhow::ensure!(
            authority == &self.authority,
            "continuation execution authority does not match the prepared run"
        );
        anyhow::ensure!(
            continuation.turn_id == self.identity.turn_id,
            "continuation turn identity does not match the prepared run"
        );
        Ok(())
    }
}

impl Deref for PreparedAgentRun {
    type Target = AgentCore;

    fn deref(&self) -> &Self::Target {
        &self.agent
    }
}

#[async_trait::async_trait]
impl FlowNodeHarness for PreparedAgentRun {
    async fn execute_flow_node(
        &self,
        request: FlowNodeExecutionRequestV1,
    ) -> anyhow::Result<FlowNodeExecutionOutcomeV1> {
        self.authority
            .validate_workspace(&request.context.workspace_root)?;
        anyhow::ensure!(
            request.context.permission_mode == self.authority.permission_mode(),
            "Flow node permission mode does not match its prepared Agent run"
        );
        anyhow::ensure!(
            request.context.sandbox_config.as_ref() == Some(self.authority.sandbox_config()),
            "Flow node sandbox does not match its prepared Agent run"
        );
        anyhow::ensure!(
            request
                .effective_capabilities
                .is_subset_of(self.authority.capability_projection()),
            "Flow node capabilities exceed its prepared Agent run"
        );
        self.agent.execute_prepared_flow_node(request).await
    }

    async fn resume_flow_node(
        &self,
        request: FlowNodeResumeRequestV1,
    ) -> anyhow::Result<FlowNodeExecutionOutcomeV1> {
        self.authority
            .validate_workspace(&request.context.workspace_root)?;
        anyhow::ensure!(
            request.context.permission_mode == self.authority.permission_mode(),
            "Flow resume permission mode does not match its prepared Agent run"
        );
        anyhow::ensure!(
            request
                .effective_capabilities
                .is_subset_of(self.authority.capability_projection()),
            "Flow resume capabilities exceed its prepared Agent run"
        );
        anyhow::ensure!(
            request.command.validates(&request.interrupt),
            "Flow ResumeCommand does not match its interrupt"
        );
        let continuation = request.interrupt.continuation.decode()?;
        self.validate_continuation(&continuation)?;
        let result = self
            .agent
            .resume_from_signal_streaming(
                continuation,
                request.command.signal.clone().into_agent_signal(),
                request
                    .context
                    .state
                    .as_ref()
                    .map(|state| Arc::clone(state.flow_session_store())),
                request.context.cancel.clone(),
                None,
            )
            .await?;
        AgentCore::flow_node_outcome_from_turn_result(result, Some(&request.interrupt))
    }
}

/// Turn input after it has been checked against a prepared run's authority.
pub struct TurnExecutionContext {
    pub(crate) input: AgentTurnInput,
    pub(crate) model_context: Option<CompiledModelContext>,
}

impl AgentCore {
    pub fn begin_run(&self, config: AgentRunConfig) -> anyhow::Result<AgentRunDraft> {
        anyhow::ensure!(
            config.identity.turn_id != Uuid::nil(),
            "Agent run turn identity cannot be nil"
        );
        anyhow::ensure!(
            config.identity.invocation_id > 0,
            "Agent run invocation identity must be positive"
        );
        let mut agent = self.clone();
        if let Some(provider) = config.provider {
            agent.apply_provider_binding(provider);
        }
        agent.apply_experience_mode(config.experience_mode);
        agent.apply_collaboration_mode(config.collaboration_mode, config.goal)?;
        agent.agent_turn_id = Some(config.identity.turn_id);
        agent.agent_depth = config.identity.path.depth().min(u8::MAX as u16) as u8;
        agent.agent_path = config.identity.path.as_str().to_string();
        agent.invocation_id = config.identity.invocation_id;
        agent.tool_host.sandbox_config = config.authority.sandbox_config().clone();
        agent.capability_projection = agent
            .capability_projection
            .intersect(config.authority.capability_projection());
        let projection = agent.capability_projection.clone();
        agent
            .enabled_bundled_plugins
            .retain(|plugin| projection.allows_plugin(plugin));
        // MCP activation is per run. Never inherit wrappers registered by a
        // previous clone; product adapters must project and sync this run's
        // explicitly allowed servers before finalization.
        agent.tool_host.active_mcp_tools.clear();
        agent.tool_host.active_connection_operations.clear();
        agent.tool_host.catalog.clear_mcp();
        if let Some(profile) = config.profile.as_ref() {
            agent.apply_agent_profile(profile);
        }
        let authority = config
            .authority
            .with_projection(agent.capability_projection.clone())?
            .with_sandbox(agent.tool_host.sandbox_config.clone())?;
        agent.execution_authority = Some(authority.clone());
        Ok(AgentRunDraft {
            agent,
            authority,
            identity: config.identity,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enterprise::CapabilityProjection;
    use crate::policy::PermissionMode;
    use crate::sandbox::LocalSandboxConfig;
    use std::path::PathBuf;

    fn authority() -> ExecutionAuthority {
        ExecutionAuthority::new(
            PathBuf::from("."),
            PermissionMode::Auto,
            LocalSandboxConfig::default(),
            CapabilityProjection::unrestricted(),
        )
        .unwrap()
    }

    #[test]
    fn draft_must_preserve_authority_before_it_can_execute() {
        let agent = AgentCore::default();
        let mut draft = agent
            .begin_run(AgentRunConfig::using_current_provider(
                authority(),
                AgentRunIdentity::root(Uuid::new_v4(), 1),
            ))
            .unwrap();
        draft.set_sandbox_config(
            LocalSandboxConfig::default().with_sandbox_mode(crate::sandbox::SandboxMode::ReadOnly),
        );
        assert!(draft.finalize().is_err());
    }

    #[test]
    fn invalid_run_identity_is_rejected_instead_of_repaired() {
        let agent = AgentCore::default();
        assert!(agent
            .begin_run(AgentRunConfig::using_current_provider(
                authority(),
                AgentRunIdentity::root(Uuid::new_v4(), 0),
            ))
            .is_err());
    }

    #[test]
    fn flow_harness_is_installed_only_during_finalization() {
        let agent = AgentCore::default();
        let draft = agent
            .begin_run(
                AgentRunConfig::using_current_provider(
                    authority(),
                    AgentRunIdentity::root(Uuid::new_v4(), 1),
                )
                .with_experience_mode(ExperienceMode::Flow),
            )
            .unwrap();
        assert!(draft.flow_harness_override.is_none());
        let prepared = draft.finalize().unwrap();
        assert!(prepared.agent.flow_harness_override.is_some());
    }

    #[test]
    fn prepared_flow_clone_attenuates_catalog_and_authority_together() {
        let agent = AgentCore::default();
        let draft = agent
            .begin_run(AgentRunConfig::using_current_provider(
                authority(),
                AgentRunIdentity::root(Uuid::new_v4(), 1),
            ))
            .unwrap();
        let prepared = draft.finalize().unwrap();
        let mut flow_agent = prepared.agent.clone();
        let narrowed = CapabilityProjection::only_tools(["library_search"]);

        flow_agent.restrict_capabilities(&narrowed);
        flow_agent
            .align_execution_authority_with_capabilities()
            .unwrap();

        assert_eq!(flow_agent.capability_projection, narrowed);
        assert_eq!(
            flow_agent
                .execution_authority
                .as_ref()
                .unwrap()
                .capability_projection(),
            &narrowed
        );
    }
}
