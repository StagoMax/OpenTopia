use anyhow::Context;
use opentopia_core::{
    BrowserAction, BrowserError, BrowserNavigateRequest, BrowserObserveOptions, BrowserRuntime,
    BrowserRuntimeConfig, BrowserSessionId, LocalBrowserRuntime,
};
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
                    "<html><head><title>OpenTopia browser smoke</title></head>",
                    "<body><h1>Browser observation contract</h1>",
                    "<button id='press' onclick=\"this.textContent='Pressed'\">Press</button>",
                    "<input id='field' aria-label='Message' /></body></html>"
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

    let mut config = BrowserRuntimeConfig::default();
    config.data_root =
        std::env::temp_dir().join(format!("opentopia-browser-smoke-{}", Uuid::new_v4()));
    config.startup_timeout = Duration::from_secs(20);
    let runtime = LocalBrowserRuntime::new(config);
    let session = BrowserSessionId::new();

    let result = async {
        runtime
            .navigate(
                session,
                BrowserNavigateRequest::new(format!("http://{address}/")),
            )
            .await?;
        let observation = runtime
            .observe(session, BrowserObserveOptions::default())
            .await?;
        let press = observation
            .nodes
            .iter()
            .find(|node| node.name == "Press")
            .context("the press button was not included in the observation")?;
        runtime
            .perform(
                session,
                observation.observation_id,
                press.node_ref,
                BrowserAction::Click,
            )
            .await?;
        match runtime
            .perform(
                session,
                observation.observation_id,
                press.node_ref,
                BrowserAction::Click,
            )
            .await
        {
            Err(BrowserError::StaleObservation { .. }) => {}
            Ok(_) => anyhow::bail!("a stale observation was accepted"),
            Err(error) => return Err(error.into()),
        }

        let refreshed = runtime
            .observe(session, BrowserObserveOptions::default())
            .await?;
        let field = refreshed
            .nodes
            .iter()
            .find(|node| node.name == "Message")
            .context("the input field was not included in the refreshed observation")?;
        runtime
            .perform(
                session,
                refreshed.observation_id,
                field.node_ref,
                BrowserAction::Type {
                    text: "OpenTopia".to_string(),
                    clear_first: true,
                },
            )
            .await?;
        let after_type = runtime
            .observe(session, BrowserObserveOptions::default())
            .await?;
        anyhow::ensure!(
            after_type.nodes.iter().any(|node| node.name == "OpenTopia"),
            "the post-action observation did not contain the typed value"
        );
        Ok::<(), anyhow::Error>(())
    }
    .await;

    let _ = runtime.close_session(session).await;
    server.abort();
    result?;
    println!("browser observation contract smoke test passed");
    Ok(())
}
