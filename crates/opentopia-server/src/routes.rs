use super::*;
use axum::routing::{get, post, put};
use axum::Router;
use tower_http::trace::TraceLayer;

/// Builds the HTTP graph from domain routers. Authentication and transport
/// middleware are applied once at the outer boundary; feature modules own only
/// their route declarations and handlers.
pub(super) fn build_router(state: AppState) -> Router {
    let cors = state.auth.cors_layer();
    let auth_state = state.clone();
    Router::new()
        .merge(platform_routes())
        .merge(conversation_routes())
        .merge(execution_routes())
        .merge(mcp_api::router())
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            auth::authorize,
        ))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn platform_routes() -> Router<AppState> {
    Router::new()
        .merge(provider_api::router())
        .merge(agent_templates_api::router())
        .merge(connections_api::router())
        .merge(contributions_api::router())
        .merge(flows_api::router())
        .merge(flow_cases_api::router())
        .merge(human_tasks_api::router())
        .merge(library_api::router())
        .merge(plugins_api::router())
        .merge(runtime_api::router())
        .merge(scm_api::router())
        .route("/health", get(health))
        .route("/api/skills", get(list_skills))
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugins/install", post(install_local_plugin))
        .route("/api/plugins/uninstall", post(uninstall_local_plugin))
        .route("/api/threads/:thread_id/plugins", put(set_thread_plugin))
}

fn conversation_routes() -> Router<AppState> {
    Router::new()
        .merge(conversation_api::router())
        .merge(events_api::router())
        .merge(interaction_api::router())
        .merge(message_api::router())
        .route("/api/threads/:thread_id/turn", get(get_turn_status))
        .route(
            "/api/threads/:thread_id/turns/:turn_id/changes",
            get(get_turn_changes),
        )
        .route(
            "/api/threads/:thread_id/turns/:turn_id/changes/preview",
            get(get_turn_file_diff_preview),
        )
        .route(
            "/api/threads/:thread_id/turns/:turn_id/undo/preview",
            post(preview_turn_undo),
        )
        .route(
            "/api/threads/:thread_id/turns/:turn_id/undo",
            post(undo_turn_changes),
        )
        .route(
            "/api/threads/:thread_id/turn/cancel",
            post(cancel_user_turn),
        )
}

fn execution_routes() -> Router<AppState> {
    Router::new()
        .merge(browser_api::router())
        .merge(context_api::router())
        .merge(resource_api::router())
        .merge(terminal_api::router())
        .merge(workspace_api::router())
        .route("/api/threads/:thread_id/git", post(run_git_workflow))
}

#[cfg(test)]
mod tests {
    #[test]
    fn process_entrypoint_does_not_assemble_services_or_routes() {
        let entrypoint = include_str!("main.rs");
        for forbidden in [
            "Router::new()",
            "ServerAgentRunScheduler::new(",
            "AgentTurnCoordinator::new(",
            "AgentCore::from_settings(",
        ] {
            assert!(
                !entrypoint.contains(forbidden),
                "main.rs must not own composition expression `{forbidden}`"
            );
        }
        assert!(include_str!("routes.rs").contains("Router::new()"));
        assert!(include_str!("bootstrap.rs").contains("ServerAgentRunScheduler::new("));
        assert!(include_str!("agent_factory.rs").contains("AgentCore::from_settings("));
    }
}
