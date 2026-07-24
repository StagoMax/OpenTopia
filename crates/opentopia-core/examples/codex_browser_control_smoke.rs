use anyhow::Context;
use opentopia_core::{
    browser_domain_approval_action, AgentCore, AgentEventPayload, AgentProfile, AgentTurnInput,
    Approval, ApprovalStatus, BrowserObserveOptions, BrowserRuntime, BrowserRuntimeConfig,
    BrowserSessionId, CodexAppServerProvider, LocalBrowserRuntime, MessagePart, PermissionMode,
    ProviderKind, ProviderSettings, SessionStore, SqliteSessionStore, ToolRegistry,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut request = [0_u8; 4096];
                let _ = socket.read(&mut request).await;
                let body = concat!(
                    "<html><head><title>OpenTopia Codex browser smoke</title></head>",
                    "<body><h1>Browser control smoke test</h1>",
                    "<button id='press' onclick=\"this.textContent='Clicked';",
                    "document.getElementById('result').textContent='Clicked'\">Press</button>",
                    "<p id='result'>Waiting</p></body></html>"
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });

    let data_root =
        std::env::temp_dir().join(format!("opentopia-codex-browser-smoke-{}", Uuid::new_v4()));
    let mut browser_config = BrowserRuntimeConfig::default();
    browser_config.data_root = data_root.clone();
    browser_config.startup_timeout = Duration::from_secs(20);
    let browser = Arc::new(LocalBrowserRuntime::new(browser_config));

    let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::open(":memory:")?);
    let workspace_root = std::env::current_dir()?;
    let thread = store.create_thread(
        Some("Codex browser control smoke".to_string()),
        workspace_root.clone(),
    )?;
    let browser_session = BrowserSessionId::from_thread(thread.id);
    let approval = Approval::pending(
        Uuid::new_v4(),
        thread.id,
        browser_domain_approval_action("127.0.0.1"),
        "Allow the smoke test's isolated local web server.",
    );
    store.insert_approval(approval.clone())?;
    store
        .update_approval_status(approval.approval_id, ApprovalStatus::Approved)?
        .context("the local browser domain approval was not persisted")?;

    let result = async {
        let mut settings = ProviderSettings::default();
        settings.kind = ProviderKind::CodexAppServer;
        settings.supports_vision = true;
        let provider = Arc::new(
            CodexAppServerProvider::from_settings(&settings)
                .context("Codex App Server provider is not configured")?,
        );
        let mut agent = AgentCore::new(provider, ToolRegistry::with_builtins());
        agent.set_browser_runtime(browser.clone());
        agent.apply_agent_profile(&AgentProfile {
            name: "browser-smoke".to_string(),
            description: "Runs a constrained browser-control smoke test.".to_string(),
            developer_instructions: "Use only the browser tool. Follow the browser observation contract exactly: navigate, observe, click using the returned observationId and nodeRef, then observe again. Do not close the browser session; the harness will clean it up.".to_string(),
            nickname_candidates: Vec::new(),
            model: None,
            model_reasoning_effort: None,
            sandbox_mode: None,
            allowed_tools: Some(vec!["browser".to_string()]),
            denied_tools: Vec::new(),
        });

        let events = agent
            .run_turn(AgentTurnInput {
                thread_id: thread.id,
                user_message_id: Uuid::new_v4(),
                workspace_root,
                content: format!(
                    "Open http://{address}/. Observe the page, click the button named Press, observe the page again, and answer only with the button's final text."
                ),
                user_content: Vec::new(),
                context_summary: None,
                conversation: Vec::new(),
                permission_mode: PermissionMode::FullAccess,
                context_budget: None,
                provider_cursor: None,
                store: Some(store.clone()),
                cancellation: None,
            })
            .await?;

        let actions = browser_actions(&events);
        for expected in ["navigate", "observe", "click"] {
            anyhow::ensure!(
                actions.iter().any(|action| action == expected),
                "Codex did not execute the required browser {expected} action; actions: {actions:?}"
            );
        }
        anyhow::ensure!(
            actions.iter().filter(|action| action.as_str() == "observe").count() >= 2,
            "Codex did not explicitly observe the page after clicking; actions: {actions:?}"
        );

        let final_observation = browser
            .observe(
                browser_session,
                BrowserObserveOptions::default(),
            )
            .await?;
        anyhow::ensure!(
            final_observation.text.contains("Clicked"),
            "the page did not reach the clicked state: {}",
            final_observation.text
        );
        let assistant_text = assistant_text(&events);
        anyhow::ensure!(
            assistant_text.trim() == "Clicked",
            "Codex did not return the requested final button text: {assistant_text:?}"
        );

        Ok::<_, anyhow::Error>((actions, assistant_text))
    }
    .await;

    let _ = browser.close_session(browser_session).await;
    server.abort();
    let _ = std::fs::remove_dir_all(&data_root);

    let (actions, assistant_text) = result?;
    println!(
        "Codex browser control smoke test passed: actions={actions:?}, final={assistant_text:?}"
    );
    Ok(())
}

fn browser_actions(events: &[AgentEventPayload]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEventPayload::ToolCallStarted { call } if call.name == "browser" => call
                .input
                .get("action")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            _ => None,
        })
        .collect()
}

fn assistant_text(events: &[AgentEventPayload]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEventPayload::AssistantMessage { message } => Some(
                message
                    .parts
                    .iter()
                    .filter_map(|part| match part {
                        MessagePart::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
