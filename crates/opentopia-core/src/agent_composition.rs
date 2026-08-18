use crate::agent::AgentCore;
use crate::agent_kernel::AgentKernel;
use crate::completion_runtime::{DefaultCompletionGate, DefaultCompletionRegistry};
use crate::context_runtime::DefaultContextAssembler;
use crate::model::ThreadModelSelection;
use crate::model_gateway::{ModelGateway, ProviderModelGateway};
use crate::prompt_runtime::AgentRuntimeSettings;
use crate::provider::{
    guardian_provider_from_settings, provider_from_settings, MockProvider, ModelProvider,
    OpenAiCompatibleProvider,
};
use crate::sandbox::LocalSandboxConfig;
use crate::settings::{
    AppSettings, ProviderSettings, ProviderToolProtocolCapabilities, RolloutBudgetSettings,
};
use crate::tool_runtime::{LocalToolRuntime, ToolRuntimeHost};
use crate::tools::ToolRegistry;
use crate::turn_inbox::BufferedTurnInbox;
use std::sync::Arc;

/// Dependency bundle used only at the Agent Core composition boundary.
///
/// Turn execution receives already-resolved ports and policies. Provider
/// selection, default implementations, and environment/settings lookup stay in
/// this module instead of leaking into the turn coordinator.
pub(crate) struct AgentCoreComposition {
    pub kernel: AgentKernel,
    pub guardian_provider: Arc<dyn ModelProvider>,
    pub tool_host: ToolRuntimeHost,
    pub rollout_budget_settings: Option<RolloutBudgetSettings>,
    pub agent_runtime_settings: AgentRuntimeSettings,
    pub provider_tool_protocol: ProviderToolProtocolCapabilities,
}

#[derive(Clone)]
pub(crate) struct AgentProviderBinding {
    pub model_gateway: Arc<dyn ModelGateway>,
    pub guardian_provider: Arc<dyn ModelProvider>,
    pub model_supports_vision: bool,
    pub provider_tool_protocol: ProviderToolProtocolCapabilities,
    pub rollout_budget_settings: Option<RolloutBudgetSettings>,
    pub agent_runtime_settings: AgentRuntimeSettings,
}

impl AgentProviderBinding {
    pub(crate) fn from_settings(
        settings: &AppSettings,
        selection: Option<&ThreadModelSelection>,
    ) -> Self {
        let connection =
            settings.provider_by_id_or_active(selection.map(|value| value.connection_id.as_str()));
        let resolved = match selection {
            Some(selection) => connection.with_model_route_override(
                Some(selection.model_id.as_str()),
                Some(selection.reasoning_effort.as_deref()),
                None,
            ),
            None => connection.clone(),
        };
        Self {
            model_gateway: Arc::new(ProviderModelGateway::from_provider(provider_from_settings(
                &resolved,
            ))),
            guardian_provider: guardian_provider_from_settings(&resolved),
            model_supports_vision: resolved.supports_vision_for_model(),
            provider_tool_protocol: resolved.capabilities().tool_protocol,
            rollout_budget_settings: resolved.rollout_budget,
            agent_runtime_settings: settings.agent_runtime.clone(),
        }
    }
}

impl AgentCoreComposition {
    fn new(
        provider: Arc<dyn ModelProvider>,
        guardian_provider: Arc<dyn ModelProvider>,
        tools: ToolRegistry,
        supports_vision: bool,
        sandbox: LocalSandboxConfig,
    ) -> Self {
        Self {
            kernel: AgentKernel::new(
                Arc::new(DefaultContextAssembler),
                Arc::new(ProviderModelGateway::from_provider(provider)),
                Arc::new(LocalToolRuntime::new()),
                Arc::new(DefaultCompletionGate),
                Arc::new(DefaultCompletionRegistry),
                Arc::new(BufferedTurnInbox::default()),
            ),
            guardian_provider,
            tool_host: ToolRuntimeHost::new(tools, supports_vision, sandbox),
            rollout_budget_settings: None,
            agent_runtime_settings: AgentRuntimeSettings::default(),
            provider_tool_protocol: ProviderToolProtocolCapabilities::default(),
        }
    }

    fn mock() -> Self {
        let provider: Arc<dyn ModelProvider> = Arc::new(MockProvider);
        Self::new(
            Arc::clone(&provider),
            provider,
            ToolRegistry::with_builtins(),
            true,
            LocalSandboxConfig::from_env(),
        )
    }

    fn from_environment() -> Self {
        let provider_settings = ProviderSettings::from_env();
        let provider: Arc<dyn ModelProvider> = OpenAiCompatibleProvider::from_env()
            .map(|provider| Arc::new(provider) as Arc<dyn ModelProvider>)
            .unwrap_or_else(|| Arc::new(MockProvider));
        let guardian_provider: Arc<dyn ModelProvider> = OpenAiCompatibleProvider::from_env()
            .map(|provider| Arc::new(provider.for_guardian()) as Arc<dyn ModelProvider>)
            .unwrap_or_else(|| Arc::new(MockProvider));
        let mut composition = Self::new(
            provider,
            guardian_provider,
            ToolRegistry::with_builtins(),
            provider_settings.supports_vision_for_model(),
            LocalSandboxConfig::from_env(),
        );
        composition.provider_tool_protocol = provider_settings.capabilities().tool_protocol;
        composition.rollout_budget_settings = provider_settings.rollout_budget;
        composition
    }

    fn from_settings(settings: &AppSettings) -> Self {
        let active = settings.active_provider();
        let mut composition = Self::new(
            provider_from_settings(active),
            guardian_provider_from_settings(active),
            ToolRegistry::with_builtins(),
            active.supports_vision_for_model(),
            settings.sandbox.to_local_sandbox_config(),
        );
        composition.rollout_budget_settings = active.rollout_budget.clone();
        composition.agent_runtime_settings = settings.agent_runtime.clone();
        composition.provider_tool_protocol = active.capabilities().tool_protocol;
        composition
    }

    fn from_provider(provider: Arc<dyn ModelProvider>, tools: ToolRegistry) -> Self {
        Self::new(
            Arc::clone(&provider),
            provider,
            tools,
            true,
            LocalSandboxConfig::from_env(),
        )
    }
}

impl Default for AgentCore {
    fn default() -> Self {
        Self::from_composition(AgentCoreComposition::mock())
    }
}

impl AgentCore {
    pub fn from_env() -> Self {
        Self::from_composition(AgentCoreComposition::from_environment())
    }

    pub fn from_settings(settings: &AppSettings) -> Self {
        Self::from_composition(AgentCoreComposition::from_settings(settings))
    }

    pub fn new(provider: Arc<dyn ModelProvider>, tools: ToolRegistry) -> Self {
        Self::from_composition(AgentCoreComposition::from_provider(provider, tools))
    }

    pub fn set_provider_from_settings(&mut self, settings: &AppSettings) {
        self.set_provider_from_settings_with_model(settings, None);
    }

    /// Resolves a thread's connection/model route at the composition boundary,
    /// then installs only normalized runtime ports into the Agent Core.
    pub fn set_provider_from_settings_with_model(
        &mut self,
        settings: &AppSettings,
        selection: Option<&ThreadModelSelection>,
    ) {
        self.apply_provider_binding(AgentProviderBinding::from_settings(settings, selection));
    }
}
