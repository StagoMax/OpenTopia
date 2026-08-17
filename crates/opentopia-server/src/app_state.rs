use super::agent_factory::AgentFactory;
use super::agent_runs::ServerAgentRunScheduler;
use super::auth::ApiAuth;
use super::library_api;
use super::turn_changes::TurnChangeManager;
use super::turns::RootTurnLifecycle;
use super::{EventBus, PtyManager, TerminalBus};
use opentopia_core::collaboration::{
    AgentCollaborationRuntime, RuntimeSnapshotDeriver, SqliteAgentActivitySource,
    SqliteCollaborationRepository,
};
use opentopia_core::mcp_host::McpExtensionHost;
use opentopia_core::{
    AgentCore, AppSettings, BackgroundProcessRegistry, BrowserRuntime, BrowserRuntimeRouter,
    CodexAccountManager, ComputerRuntime, SqliteSessionStore, TurnInbox,
};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::mpsc;
use uuid::Uuid;

/// Process-scoped capabilities shared by HTTP adapters.
///
/// Construction is owned by `bootstrap`; route declarations are owned by
/// `routes`. Keeping the state contract separate prevents either module from
/// becoming the owner of business handlers.
#[derive(Clone)]
pub(super) struct AppState {
    pub(super) store: Arc<SqliteSessionStore>,
    pub(super) agent: Arc<RwLock<AgentCore>>,
    pub(super) agent_factory: AgentFactory,
    pub(super) settings: Arc<RwLock<AppSettings>>,
    pub(super) codex_account: Arc<CodexAccountManager>,
    pub(super) events: EventBus,
    pub(super) terminals: TerminalBus,
    pub(super) ptys: PtyManager,
    pub(super) browser: Arc<dyn BrowserRuntime>,
    pub(super) browser_router: Arc<BrowserRuntimeRouter>,
    pub(super) computer: Arc<dyn ComputerRuntime>,
    pub(super) mcp_host: McpExtensionHost,
    pub(super) auth: ApiAuth,
    pub(super) turns: RootTurnLifecycle,
    pub(super) turn_changes: TurnChangeManager,
    pub(super) turn_queue: mpsc::UnboundedSender<Uuid>,
    pub(super) turn_inbox: Arc<dyn TurnInbox>,
    pub(super) collaboration_repository: Arc<SqliteCollaborationRepository>,
    pub(super) collaboration_runtime: AgentCollaborationRuntime,
    pub(super) agent_activity: Arc<SqliteAgentActivitySource>,
    pub(super) snapshot_deriver: Arc<dyn RuntimeSnapshotDeriver>,
    pub(super) agent_run_scheduler: Arc<ServerAgentRunScheduler>,
    pub(super) background: BackgroundProcessRegistry,
    pub(super) app_views: Arc<Mutex<opentopia_core::AppViewHost>>,
    pub(super) library_providers: Arc<library_api::LibraryProviderRegistry>,
}
