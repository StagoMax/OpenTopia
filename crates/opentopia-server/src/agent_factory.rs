use super::turn_changes::TurnChangeManager;
use opentopia_core::{
    AgentCore, AppSettings, BackgroundProcessRegistry, BrowserRuntime, ComputerRuntime, TurnInbox,
};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{info, warn};

/// Rebuildable process-scoped Agent composition.
///
/// Startup and settings hot-reload must use this same factory so shared Inbox,
/// background jobs, device runtimes, and mutation observation cannot drift.
#[derive(Clone)]
pub(super) struct AgentFactory {
    turn_inbox: Arc<dyn TurnInbox>,
    browser: Arc<dyn BrowserRuntime>,
    computer: Arc<dyn ComputerRuntime>,
    background: BackgroundProcessRegistry,
    turn_changes: TurnChangeManager,
}

impl AgentFactory {
    pub(super) fn new(
        turn_inbox: Arc<dyn TurnInbox>,
        browser: Arc<dyn BrowserRuntime>,
        computer: Arc<dyn ComputerRuntime>,
        background: BackgroundProcessRegistry,
        turn_changes: TurnChangeManager,
    ) -> Self {
        Self {
            turn_inbox,
            browser,
            computer,
            background,
            turn_changes,
        }
    }

    pub(super) fn build(&self, settings: &AppSettings) -> AgentCore {
        let mut agent = AgentCore::from_settings(settings).with_turn_inbox(self.turn_inbox.clone());
        agent.set_browser_runtime(self.browser.clone());
        agent.set_computer_runtime(self.computer.clone());
        agent.set_background_processes(self.background.clone());
        agent.set_file_mutation_observer(Arc::new(self.turn_changes.clone()));
        apply_process_tool_policy(&mut agent);
        agent
    }
}

/// Applies a process-wide tool allowlist supplied by the trusted launcher. The
/// allowlist is never accepted from a chat request.
fn apply_process_tool_policy(agent: &mut AgentCore) {
    let Some(raw) = std::env::var("OPENTOPIA_TOOL_ALLOWLIST")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let tools = raw
        .split(',')
        .map(str::trim)
        .filter(|tool| !tool.is_empty())
        .map(str::to_string)
        .collect::<HashSet<_>>();
    if tools.is_empty() {
        warn!(
            "OPENTOPIA_TOOL_ALLOWLIST did not contain any tool names; ignoring process tool policy"
        );
        return;
    }
    info!(tools = ?tools, "restricting agent tools for this process");
    agent.restrict_to_tools(tools);
}
