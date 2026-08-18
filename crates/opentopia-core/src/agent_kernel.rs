use crate::completion_runtime::{CompletionGate, CompletionRegistry};
use crate::context_runtime::ContextAssembler;
use crate::model_gateway::ModelGateway;
use crate::tool_runtime::ToolRuntime;
use crate::turn_inbox::TurnInbox;
use std::sync::Arc;

/// Immutable, process-scoped ports shared by prepared Agent runs.
///
/// Provider selection creates a derived kernel with a bound model gateway;
/// sandbox authority, capability projections, and invocation identity remain
/// outside the kernel and are validated for each run.
#[derive(Clone)]
pub struct AgentKernel {
    pub(crate) context_assembler: Arc<dyn ContextAssembler>,
    pub(crate) model_gateway: Arc<dyn ModelGateway>,
    pub(crate) tool_runtime: Arc<dyn ToolRuntime>,
    pub(crate) completion_gate: Arc<dyn CompletionGate>,
    pub(crate) completion_registry: Arc<dyn CompletionRegistry>,
    pub(crate) turn_inbox: Arc<dyn TurnInbox>,
}

impl AgentKernel {
    pub(crate) fn new(
        context_assembler: Arc<dyn ContextAssembler>,
        model_gateway: Arc<dyn ModelGateway>,
        tool_runtime: Arc<dyn ToolRuntime>,
        completion_gate: Arc<dyn CompletionGate>,
        completion_registry: Arc<dyn CompletionRegistry>,
        turn_inbox: Arc<dyn TurnInbox>,
    ) -> Self {
        Self {
            context_assembler,
            model_gateway,
            tool_runtime,
            completion_gate,
            completion_registry,
            turn_inbox,
        }
    }

    pub(crate) fn with_model_gateway(&self, gateway: Arc<dyn ModelGateway>) -> Self {
        let mut kernel = self.clone();
        kernel.model_gateway = gateway;
        kernel
    }

    pub(crate) fn with_context_assembler(&self, assembler: Arc<dyn ContextAssembler>) -> Self {
        let mut kernel = self.clone();
        kernel.context_assembler = assembler;
        kernel
    }

    pub(crate) fn with_tool_runtime(&self, runtime: Arc<dyn ToolRuntime>) -> Self {
        let mut kernel = self.clone();
        kernel.tool_runtime = runtime;
        kernel
    }

    pub(crate) fn with_completion_gate(&self, gate: Arc<dyn CompletionGate>) -> Self {
        let mut kernel = self.clone();
        kernel.completion_gate = gate;
        kernel
    }

    pub(crate) fn with_completion_registry(&self, registry: Arc<dyn CompletionRegistry>) -> Self {
        let mut kernel = self.clone();
        kernel.completion_registry = registry;
        kernel
    }

    pub(crate) fn with_turn_inbox(&self, inbox: Arc<dyn TurnInbox>) -> Self {
        let mut kernel = self.clone();
        kernel.turn_inbox = inbox;
        kernel
    }
}
