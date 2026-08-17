use super::*;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, patch, post, put};
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
        .merge(integration_routes())
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
        .merge(agent_templates_api::router())
        .merge(contributions_api::router())
        .merge(flows_api::router())
        .merge(library_api::router())
        .merge(plugins_api::router())
        .merge(scm_api::router())
        .route("/health", get(health))
        .route("/api/settings", get(get_settings).patch(update_settings))
        .route(
            "/api/sandbox/windows/setup",
            get(get_windows_sandbox_setup)
                .post(configure_windows_sandbox)
                .delete(remove_windows_sandbox_configuration),
        )
        .route("/api/skills", get(list_skills))
        .route("/api/plugins", get(list_plugins))
        .route("/api/plugins/install", post(install_local_plugin))
        .route("/api/plugins/uninstall", post(uninstall_local_plugin))
        .route("/api/threads/:thread_id/plugins", put(set_thread_plugin))
        .route("/api/provider/drivers", get(list_provider_drivers))
        .route("/api/provider/health", get(provider_health))
        .route("/api/provider/test", post(test_provider_connection))
        .route("/api/codex/account", get(get_codex_account))
        .route("/api/codex/account/login", post(start_codex_login))
        .route("/api/codex/account/login/cancel", post(cancel_codex_login))
        .route("/api/codex/account/logout", post(logout_codex_account))
        .route(
            "/api/provider/:provider_id/models/sync",
            post(sync_provider_models),
        )
        .route("/api/threads/:thread_id/model", put(set_thread_model))
}

fn conversation_routes() -> Router<AppState> {
    Router::new()
        .route("/api/threads", get(list_threads).post(create_thread))
        .route("/api/threads/:thread_id/title", post(generate_thread_title))
        .route(
            "/api/threads/:thread_id",
            patch(update_thread).delete(delete_thread),
        )
        .route("/api/projects", get(list_projects).post(create_project))
        .route(
            "/api/projects/:project_id",
            patch(update_project).delete(delete_project),
        )
        .route(
            "/api/threads/:thread_id/messages",
            get(list_messages)
                .post(send_message)
                .layer(DefaultBodyLimit::max(MAX_INLINE_IMAGE_BYTES * 5)),
        )
        .route("/api/threads/:thread_id/events", get(list_events))
        .route("/api/threads/:thread_id/events/stream", get(stream_events))
        .route("/api/threads/:thread_id/agents", get(list_agent_threads))
        .route(
            "/api/threads/:thread_id/agents/:agent_thread_id/interrupt",
            post(interrupt_agent_thread),
        )
        .route(
            "/api/threads/:thread_id/agents/events/stream",
            get(stream_agent_events),
        )
        .route("/api/threads/:thread_id/goal", get(get_thread_goal))
        .route("/api/threads/:thread_id/goal/:goal_id", patch(update_goal))
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
        .route("/api/threads/:thread_id/approvals", get(list_approvals))
        .route(
            "/api/threads/:thread_id/approvals/:approval_id/decision",
            post(decide_approval),
        )
        .route(
            "/api/threads/:thread_id/user-input",
            get(list_user_input_requests),
        )
        .route(
            "/api/threads/:thread_id/user-input/:request_id/response",
            post(respond_to_user_input),
        )
        .route(
            "/api/threads/:thread_id/turns/:turn_id/external-action/resume",
            post(resume_external_action),
        )
}

fn execution_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/threads/:thread_id/terminal/commands",
            post(start_terminal_command),
        )
        .route(
            "/api/threads/:thread_id/terminal/cancel",
            post(cancel_terminal_command),
        )
        .route(
            "/api/threads/:thread_id/terminal/history",
            get(list_terminal_history),
        )
        .route(
            "/api/threads/:thread_id/terminal/stream",
            get(stream_terminal_events),
        )
        .route(
            "/api/threads/:thread_id/terminal/session",
            get(get_terminal_session).post(ensure_terminal_session),
        )
        .route(
            "/api/threads/:thread_id/terminal/session/input",
            post(write_terminal_session),
        )
        .route(
            "/api/threads/:thread_id/terminal/session/resize",
            post(resize_terminal_session),
        )
        .route(
            "/api/threads/:thread_id/terminal/session/close",
            post(close_terminal_session),
        )
        .route(
            "/api/threads/:thread_id/workspace/tree",
            get(list_workspace_tree),
        )
        .route(
            "/api/threads/:thread_id/workspace/file",
            get(read_workspace_file),
        )
        .route(
            "/api/threads/:thread_id/workspace/search",
            post(search_workspace),
        )
        .route(
            "/api/threads/:thread_id/workspace/diff",
            get(get_workspace_diff),
        )
        .route(
            "/api/threads/:thread_id/workspace/diff/revert",
            post(revert_workspace_file),
        )
        .route(
            "/api/threads/:thread_id/workspace/diff/hunk",
            post(apply_workspace_diff_hunk),
        )
        .route("/api/threads/:thread_id/sandbox", get(get_sandbox))
        .route("/api/threads/:thread_id/browser", post(run_browser_command))
        .route(
            "/api/threads/:thread_id/browser/runtime",
            get(get_browser_runtime).post(bind_browser_runtime),
        )
        .route(
            "/api/threads/:thread_id/computer/windows",
            get(list_computer_windows),
        )
        .route(
            "/api/threads/:thread_id/computer/observe",
            post(observe_computer_window),
        )
        .route(
            "/api/threads/:thread_id/computer/session",
            post(close_computer_session),
        )
        .route("/api/threads/:thread_id/git", post(run_git_workflow))
        .route("/api/threads/:thread_id/context", get(get_context_status))
        .route(
            "/api/threads/:thread_id/context/compact",
            post(compact_context),
        )
        .route("/api/threads/:thread_id/trajectory", get(export_trajectory))
        .route("/api/threads/:thread_id/artifacts", get(list_artifacts))
        .route(
            "/api/threads/:thread_id/artifacts/:artifact_id",
            get(get_artifact),
        )
        .route(
            "/api/threads/:thread_id/previews/resolve",
            post(resolve_preview),
        )
        .route(
            "/api/threads/:thread_id/previews/:preview_id/content",
            get(read_preview_content),
        )
        .route(
            "/api/threads/:thread_id/previews/:preview_id/workbook",
            get(get_preview_workbook),
        )
        .route(
            "/api/threads/:thread_id/previews/:preview_id/range",
            get(read_preview_range),
        )
        .route(
            "/api/threads/:thread_id/resources/resolve",
            post(resolve_preview),
        )
        .route(
            "/api/threads/:thread_id/resources/:preview_id",
            get(get_resource_metadata).delete(release_resource),
        )
        .route(
            "/api/threads/:thread_id/resources/:preview_id/content",
            get(read_preview_content)
                .put(write_resource_content)
                .layer(DefaultBodyLimit::max(MAX_PREVIEW_CONTENT_BYTES as usize)),
        )
        .route(
            "/api/threads/:thread_id/resources/:preview_id/workbook",
            get(get_preview_workbook),
        )
        .route(
            "/api/threads/:thread_id/resources/:preview_id/range",
            get(read_preview_range),
        )
}

fn integration_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/mcp/servers",
            get(list_mcp_servers).post(create_mcp_server),
        )
        .route(
            "/api/mcp/servers/:server_id",
            patch(update_mcp_server).delete(delete_mcp_server),
        )
        .route(
            "/api/mcp/servers/:server_id/restart",
            post(restart_mcp_server),
        )
        .route("/api/mcp/servers/:server_id/tools", get(list_mcp_tools))
        .route("/api/mcp/servers/:server_id/call-tool", post(call_mcp_tool))
        .route("/api/threads/:thread_id/mcp", get(list_thread_mcp_servers))
        .route(
            "/api/threads/:thread_id/mcp/:server_id",
            put(set_thread_mcp_server),
        )
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
