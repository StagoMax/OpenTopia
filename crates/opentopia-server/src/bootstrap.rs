use super::agent_factory::AgentFactory;
use super::agent_runs::ServerAgentRunScheduler;
use super::agent_turn_coordinator::AgentTurnCoordinator;
use super::auth::ApiAuth;
use super::turn_changes::TurnChangeManager;
use super::turns::RootTurnLifecycle;
use super::{
    launch_next_queued_turn, library_api, plugins_api, routes, AppState, Args, EventBus,
    PtyManager, TerminalBus,
};
use opentopia_core::collaboration::{
    AgentCollaborationRuntime, AgentMailboxNotifier, AgentRunCommand, AgentRunScheduler,
    AgentTurnStatus, AttenuatingRuntimeSnapshotDeriver, CollaborationRegistry,
    RuntimeSnapshotDeriver, SqliteAgentActivitySource, SqliteCollaborationRepository,
};
use opentopia_core::mcp_host::McpExtensionHost;
use opentopia_core::{
    bundled_plugins_path, compact_database_copy, ensure_bundled_plugins_installed, AppSettings,
    BackgroundProcessRegistry, BrowserRuntime, BrowserRuntimeConfig, BrowserRuntimeRouter,
    BufferedTurnInbox, ChromeExtensionBrowserRuntime, ChromeExtensionBrowserRuntimeConfig,
    CodexAccountManager, ComputerRuntime, ComputerRuntimeConfig, DesktopBrowserRuntime,
    LocalBrowserRuntime, LocalComputerRuntime, LocalExecutionEnvironment, SessionStore,
    SqliteSessionStore, TurnInbox,
};
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct CollaborationRecoveryReport {
    pub(super) interrupted: usize,
    pub(super) resubmitted: usize,
}

pub(super) async fn recover_collaboration_runs(
    repository: &SqliteCollaborationRepository,
    run_scheduler: &dyn AgentRunScheduler,
    mailbox_notifier: &dyn AgentMailboxNotifier,
    activity: &SqliteAgentActivitySource,
) -> anyhow::Result<CollaborationRecoveryReport> {
    let mut report = CollaborationRecoveryReport::default();
    for turn in repository.list_recoverable_turns()? {
        let thread = repository.get_thread(turn.agent_thread_id).await?;
        // A root Turn is driven by the product request adapter because it owns
        // UI continuation and change-capture context. After a restart that
        // adapter cannot safely reconstruct a queued/running invocation, so the
        // canonical root is interrupted. Descendant queued work is resumable by
        // the Agent run scheduler from its frozen snapshot.
        if thread.parent_agent_thread_id.is_none() || turn.status == AgentTurnStatus::Running {
            if let Some(message) = repository.record_turn_state(
                turn.id,
                AgentTurnStatus::Interrupted,
                &json!({
                    "status": "interrupted",
                    "reason": "server restarted without a resumable checkpoint",
                    "agentTurnId": turn.id,
                }),
            )? {
                mailbox_notifier.message_enqueued(&message);
            }
            activity.notify(turn.agent_thread_id);
            report.interrupted += 1;
        } else {
            run_scheduler
                .submit(AgentRunCommand::Start {
                    session_id: turn.session_id,
                    agent_thread_id: turn.agent_thread_id,
                    agent_turn_id: turn.id,
                })
                .await?;
            report.resubmitted += 1;
        }
    }
    Ok(report)
}

/// Owns process-level dependency assembly and lifecycle startup.
///
/// HTTP handlers receive the completed `AppState`; they do not construct
/// providers, execution runtimes, collaboration services, or background
/// workers themselves.
pub(super) async fn run(args: Args) -> anyhow::Result<()> {
    if let Some(output) = args.compact_db_output.as_ref() {
        let trace_archive = args
            .compact_trace_archive
            .clone()
            .unwrap_or_else(|| default_trace_archive_path(output));
        let report = compact_database_copy(&args.db, output, &trace_archive)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let state = assemble_application(&args).await?;
    start_recovery_workers(&state).await?;

    let app = routes::build_router(state);
    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, db = %args.db.display(), "OpenTopia server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn assemble_application(args: &Args) -> anyhow::Result<AppState> {
    let auth = ApiAuth::from_env()?;
    for outcome in ensure_bundled_plugins_installed(&bundled_plugins_path())? {
        info!(
            plugin = %outcome.name,
            version = %outcome.version,
            status = ?outcome.status,
            path = %outcome.path.display(),
            "bundled plugin package ready"
        );
    }

    let store = Arc::new(SqliteSessionStore::open(&args.db)?);
    plugins_api::ensure_default_bundled_plugin_permissions(&store)?;
    recover_persistent_state(&store)?;

    let loaded_settings = store.load_settings(args.permission)?;
    let settings = Arc::new(RwLock::new(loaded_settings.clone()));
    let mcp_host = assemble_mcp_host(Arc::clone(&store), Arc::clone(&settings)).await;
    let browser = initialize_browser_runtime().await;
    let browser_runtime: Arc<dyn BrowserRuntime> = browser.clone();
    let computer: Arc<dyn ComputerRuntime> =
        Arc::new(LocalComputerRuntime::new(ComputerRuntimeConfig::default()));
    let background = BackgroundProcessRegistry::default();
    let turn_inbox: Arc<dyn TurnInbox> = Arc::new(BufferedTurnInbox::default());
    let turn_changes = TurnChangeManager::new(Arc::clone(&store));
    let agent_factory = AgentFactory::new(
        Arc::clone(&turn_inbox),
        Arc::clone(&browser_runtime),
        Arc::clone(&computer),
        background.clone(),
        turn_changes.clone(),
    );
    let agent = Arc::new(RwLock::new(agent_factory.build(&loaded_settings)));

    let collaboration_repository =
        Arc::new(SqliteCollaborationRepository::new(Arc::clone(&store))?);
    let agent_activity = Arc::new(SqliteAgentActivitySource::new(Arc::clone(
        &collaboration_repository,
    )));
    let snapshot_deriver: Arc<dyn RuntimeSnapshotDeriver> =
        Arc::new(AttenuatingRuntimeSnapshotDeriver);
    let agent_run_scheduler = ServerAgentRunScheduler::new(
        Arc::clone(&collaboration_repository),
        Arc::clone(&turn_inbox),
        12,
    );
    let collaboration_runtime = AgentCollaborationRuntime::new(
        collaboration_repository.clone(),
        agent_run_scheduler.clone(),
        collaboration_repository.clone(),
    )
    .with_mailbox_notifier(agent_run_scheduler.clone());
    let turns = RootTurnLifecycle::new(
        Arc::clone(&store),
        Arc::clone(&collaboration_repository),
        Arc::clone(&agent_activity),
        agent_run_scheduler.clone(),
    );
    let agent_turn_coordinator = Arc::new(AgentTurnCoordinator::new(
        Arc::clone(&store),
        Arc::clone(&collaboration_repository),
        Arc::clone(&agent_activity),
        collaboration_runtime.clone(),
        Arc::clone(&snapshot_deriver),
        agent_run_scheduler.clone(),
        Arc::clone(&agent),
        Arc::clone(&settings),
        mcp_host.clone(),
        Arc::clone(&turn_inbox),
    ));
    agent_run_scheduler.start(agent_turn_coordinator);

    let (turn_queue, queued_threads) = mpsc::unbounded_channel();
    let state = AppState {
        store: Arc::clone(&store),
        agent,
        agent_factory,
        settings,
        codex_account: Arc::new(CodexAccountManager::default()),
        events: EventBus::default(),
        terminals: TerminalBus::default(),
        ptys: PtyManager::default(),
        browser: browser_runtime,
        browser_router: browser,
        computer,
        mcp_host,
        auth,
        turns,
        turn_changes,
        turn_queue,
        turn_inbox,
        collaboration_repository,
        collaboration_runtime,
        agent_activity,
        snapshot_deriver,
        agent_run_scheduler,
        background,
        app_views: Arc::new(Mutex::new(opentopia_core::AppViewHost::default())),
        library_providers: Arc::new(library_api::LibraryProviderRegistry::from_env()?),
        resources: crate::resource_registry::ResourceRegistry::default(),
    };
    spawn_turn_queue_worker(state.clone(), queued_threads);
    Ok(state)
}

fn recover_persistent_state(store: &SqliteSessionStore) -> anyhow::Result<()> {
    let indeterminate_effects = store.mark_running_effects_indeterminate()?;
    if indeterminate_effects > 0 {
        warn!(
            indeterminate_effects,
            "marked in-flight effects indeterminate for reconciliation"
        );
    }
    Ok(())
}

async fn assemble_mcp_host(
    store: Arc<SqliteSessionStore>,
    settings: Arc<RwLock<AppSettings>>,
) -> McpExtensionHost {
    let mcp_host = McpExtensionHost::with_execution_environment_factory(move |config| {
        let workspace_root = config
            .cwd
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let sandbox_config = settings
            .read()
            .expect("settings lock poisoned")
            .sandbox
            .to_local_sandbox_config();
        Arc::new(LocalExecutionEnvironment::with_sandbox_config(
            workspace_root,
            sandbox_config,
        ))
    })
    .with_tool_catalog_store(store.clone());
    match mcp_host.warm_tool_cache().await {
        Ok(0) => {}
        Ok(tools) => info!(tools, "restored persisted MCP tool schema cache"),
        Err(err) => warn!(?err, "failed to restore persisted MCP tool schema cache"),
    }
    restore_enabled_mcp_servers(store, mcp_host.clone());
    mcp_host
}

fn spawn_turn_queue_worker(state: AppState, mut queued_threads: mpsc::UnboundedReceiver<Uuid>) {
    tokio::spawn(async move {
        while let Some(thread_id) = queued_threads.recv().await {
            launch_next_queued_turn(&state, thread_id);
        }
    });
}

async fn start_recovery_workers(state: &AppState) -> anyhow::Result<()> {
    recover_collaboration_runs(
        state.collaboration_repository.as_ref(),
        state.agent_run_scheduler.as_ref(),
        state.agent_run_scheduler.as_ref(),
        state.agent_activity.as_ref(),
    )
    .await?;
    let interrupted_projections = state.store.interrupt_active_turns()?;
    if interrupted_projections > 0 {
        info!(
            interrupted_projections,
            "reconciled root product Turn projections after canonical AgentTurn recovery"
        );
    }
    for thread in state.store.list_threads_including_archived(true)? {
        if !state.store.list_queued_turn_messages(thread.id)?.is_empty() {
            let _ = state.turn_queue.send(thread.id);
        }
    }
    Ok(())
}

fn default_trace_archive_path(output: &std::path::Path) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("opentopia-compact");
    output.with_file_name(format!("{stem}.traces.zip"))
}

fn restore_enabled_mcp_servers(store: Arc<SqliteSessionStore>, host: McpExtensionHost) {
    tokio::spawn(async move {
        let servers = match store.list_mcp_servers() {
            Ok(servers) => servers,
            Err(err) => {
                tracing::error!(?err, "failed to restore MCP server configuration");
                return;
            }
        };
        for server in servers.into_iter().filter(|server| server.enabled) {
            let server_id = server.server_id;
            if let Err(err) = host.ensure_server(server).await {
                warn!(?err, %server_id, "failed to restore MCP server");
            }
        }
    });
}

async fn initialize_browser_runtime() -> Arc<BrowserRuntimeRouter> {
    let broker_url = std::env::var("OPENTOPIA_DESKTOP_BROWSER_BROKER_URL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let broker_token = std::env::var("OPENTOPIA_DESKTOP_BROWSER_BROKER_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());

    match (broker_url, broker_token) {
        (Some(url), Some(token)) => match DesktopBrowserRuntime::new(&url, &token) {
            Ok(runtime) => match runtime.health_check().await {
                Ok(()) => {
                    info!("using the Electron desktop browser broker");
                    let managed: Arc<dyn BrowserRuntime> = Arc::new(runtime);
                    return Arc::new(BrowserRuntimeRouter::new(
                        managed,
                        initialize_chrome_browser_runtime().await,
                    ));
                }
                Err(error) => {
                    warn!(%error, "desktop browser broker health check failed; using local browser runtime");
                }
            },
            Err(error) => {
                warn!(%error, "desktop browser broker configuration is invalid; using local browser runtime");
            }
        },
        (None, None) => {
            info!("desktop browser broker is not configured; using local browser runtime");
        }
        _ => {
            warn!(
                "desktop browser broker requires both URL and token; using local browser runtime"
            );
        }
    }

    let mut config = BrowserRuntimeConfig::default();
    if let Some(data_root) =
        std::env::var_os("OPENTOPIA_BROWSER_DATA_ROOT").filter(|value| !value.is_empty())
    {
        config.data_root = PathBuf::from(data_root);
        info!(path = %config.data_root.display(), "using configured local browser data root");
    }
    Arc::new(BrowserRuntimeRouter::new(
        Arc::new(LocalBrowserRuntime::new(config)),
        initialize_chrome_browser_runtime().await,
    ))
}

async fn initialize_chrome_browser_runtime() -> Option<Arc<dyn BrowserRuntime>> {
    let url = std::env::var("OPENTOPIA_CHROME_BRIDGE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let token = std::env::var("OPENTOPIA_CHROME_BRIDGE_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let (Some(url), Some(token)) = (url, token) else {
        info!("Chrome extension bridge is not configured");
        return None;
    };
    let runtime = match ChromeExtensionBrowserRuntime::new(ChromeExtensionBrowserRuntimeConfig {
        bridge_url: url,
        bridge_token: token,
        browser: BrowserRuntimeConfig::default(),
    }) {
        Ok(runtime) => runtime,
        Err(error) => {
            warn!(%error, "Chrome extension browser configuration is invalid");
            return None;
        }
    };
    match runtime.health_check().await {
        Ok(()) => {
            info!("Chrome extension browser bridge is available");
            Some(Arc::new(runtime))
        }
        Err(error) => {
            warn!(%error, "Chrome extension browser bridge health check failed");
            None
        }
    }
}
