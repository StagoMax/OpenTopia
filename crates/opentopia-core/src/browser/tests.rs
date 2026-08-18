use super::cdp_transport::CdpPage;
use super::io_support::{content_type_for_path, png_looks_blank};
use super::process::{browser_profile_storage_root, discover_browser_executable};
use super::{
    truncate_utf8, BrowserAction, BrowserBackendKind, BrowserContent, BrowserDownloadRequest,
    BrowserError, BrowserNavigateRequest, BrowserNetworkGrant, BrowserObserveOptions,
    BrowserProfileId, BrowserProfilePersistence, BrowserRuntime, BrowserRuntimeConfig,
    BrowserSessionId, BrowserSessionSpec, BrowserSurfaceKind, LocalBrowserRuntime,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message as WebSocketMessage};
use uuid::Uuid;

#[test]
fn browser_profile_storage_preserves_legacy_default_and_isolates_named_profiles() {
    let config = BrowserRuntimeConfig {
        data_root: PathBuf::from("browser-data"),
        ..BrowserRuntimeConfig::default()
    };
    let session = BrowserSessionId::new();
    let default_spec = BrowserSessionSpec::from(session);
    assert_eq!(
        browser_profile_storage_root(&config, &default_spec),
        PathBuf::from("browser-data")
    );

    let named = BrowserSessionSpec::persistent(session, BrowserProfileId::new("work").unwrap());
    assert_eq!(
        browser_profile_storage_root(&config, &named),
        PathBuf::from("browser-data").join("profiles").join("work")
    );
    let ephemeral = BrowserSessionSpec {
        profile_persistence: BrowserProfilePersistence::Ephemeral,
        ..named
    };
    assert_eq!(
        browser_profile_storage_root(&config, &ephemeral),
        PathBuf::from("browser-data").join("ephemeral").join("work")
    );
}

#[test]
fn local_runtime_advertises_managed_backend_guarantees() {
    let runtime = LocalBrowserRuntime::new(BrowserRuntimeConfig::default());
    let capabilities = runtime.capabilities();
    assert_eq!(capabilities.backend, BrowserBackendKind::LocalChrome);
    assert_eq!(capabilities.surface, BrowserSurfaceKind::Headless);
    assert!(capabilities.hard_network_isolation);
    assert!(!capabilities.supports_external_profile);
    assert!(capabilities
        .profile_persistence
        .contains(&BrowserProfilePersistence::Ephemeral));
}

#[test]
fn url_validation_is_scheme_bounded() {
    let runtime = LocalBrowserRuntime::new(BrowserRuntimeConfig::default());
    assert!(runtime.validate_url("https://example.com/a").is_ok());
    assert!(matches!(
        runtime.validate_url("file:///etc/passwd"),
        Err(BrowserError::DisallowedScheme(_))
    ));
    assert!(matches!(
        runtime.validate_url("not a url"),
        Err(BrowserError::InvalidUrl(_))
    ));
}

#[test]
fn utf8_truncation_keeps_valid_boundaries() {
    let (value, truncated) = truncate_utf8("ab你好", 4);
    assert_eq!(value, "ab");
    assert!(truncated);
}

#[test]
fn download_content_types_are_inferred_for_common_files() {
    assert_eq!(
        content_type_for_path(Path::new("report.pdf")),
        Some("application/pdf".to_string())
    );
    assert_eq!(content_type_for_path(Path::new("report.unknown")), None);
}

#[test]
fn screenshot_pixel_validation_distinguishes_black_from_rendered_content() {
    fn image(red: u8, green: u8, blue: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let pixel = [red, green, blue, 255];
            writer.write_image_data(&pixel.repeat(4)).unwrap();
        }
        bytes
    }

    assert!(png_looks_blank(&image(0, 0, 0)).unwrap());
    assert!(!png_looks_blank(&image(24, 120, 220)).unwrap());
}

#[tokio::test]
async fn cdp_connection_correlates_responses_and_routes_session_events() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let command = socket.next().await.unwrap().unwrap();
        let WebSocketMessage::Text(command) = command else {
            panic!("expected a text command");
        };
        let command: Value = serde_json::from_str(&command).unwrap();
        assert_eq!(command["sessionId"], "session-a");
        socket
            .send(WebSocketMessage::Text(
                json!({
                    "method": "Page.frameNavigated",
                    "sessionId": "session-b",
                    "params": { "frame": { "url": "https://ignored.example" } }
                })
                .to_string(),
            ))
            .await
            .unwrap();
        socket
            .send(WebSocketMessage::Text(
                json!({
                    "method": "Page.frameNavigated",
                    "sessionId": "session-a",
                    "params": { "frame": { "url": "https://example.com" } }
                })
                .to_string(),
            ))
            .await
            .unwrap();
        socket
            .send(WebSocketMessage::Text(
                json!({ "id": command["id"], "sessionId": "session-a", "result": { "ok": true } })
                    .to_string(),
            ))
            .await
            .unwrap();
    });

    let mut page = CdpPage::connect(&format!("ws://{address}"), Duration::from_secs(2))
        .await
        .unwrap();
    page.session_id = Some("session-a".to_string());
    let response = page.command("Runtime.evaluate", json!({})).await.unwrap();
    assert_eq!(response["ok"], true);
    let event = page.next_event().await.unwrap().unwrap();
    assert_eq!(event.session_id.as_deref(), Some("session-a"));
    assert_eq!(
        event.params.pointer("/frame/url").and_then(Value::as_str),
        Some("https://example.com")
    );
    server.await.unwrap();
    for _ in 0..20 {
        if !page.is_connected() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!page.is_connected());
}

#[tokio::test]
async fn cdp_connection_enforces_network_grants_inside_the_actor() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let command = socket.next().await.unwrap().unwrap();
        let WebSocketMessage::Text(command) = command else {
            panic!("expected a text command");
        };
        let command: Value = serde_json::from_str(&command).unwrap();

        for (request_id, url, expected_method) in [
            (
                "allowed",
                "https://example.com/app.js",
                "Fetch.continueRequest",
            ),
            (
                "blocked",
                "https://tracker.example/pixel",
                "Fetch.failRequest",
            ),
        ] {
            socket
                .send(WebSocketMessage::Text(
                    json!({
                        "method": "Fetch.requestPaused",
                        "sessionId": "session-a",
                        "params": {
                            "requestId": request_id,
                            "request": { "url": url },
                            "resourceType": "Script"
                        }
                    })
                    .to_string(),
                ))
                .await
                .unwrap();
            let interception = socket.next().await.unwrap().unwrap();
            let WebSocketMessage::Text(interception) = interception else {
                panic!("expected a text interception command");
            };
            let interception: Value = serde_json::from_str(&interception).unwrap();
            assert_eq!(interception["method"], expected_method);
            assert_eq!(interception["params"]["requestId"], request_id);
            assert_eq!(interception["sessionId"], "session-a");
        }

        socket
            .send(WebSocketMessage::Text(
                json!({ "id": command["id"], "result": { "ok": true } }).to_string(),
            ))
            .await
            .unwrap();
    });

    let mut page = CdpPage::connect(&format!("ws://{address}"), Duration::from_secs(2))
        .await
        .unwrap();
    page.session_id = Some("session-a".to_string());
    page.grant_network_access(BrowserNetworkGrant::new(["example.com"]).unwrap())
        .unwrap();
    let response = page.command("Runtime.evaluate", json!({})).await.unwrap();
    assert_eq!(response["ok"], true);
    let blocked = page.next_event().await.unwrap().unwrap();
    assert_eq!(blocked.method, "OpenTopia.networkRequestBlocked");
    assert_eq!(blocked.params["host"], "tracker.example");
    server.await.unwrap();
}

#[tokio::test]
async fn local_chromium_runtime_smoke_test() {
    if discover_browser_executable(None).is_err() {
        return;
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut request = [0_u8; 4096];
                let read = socket.read(&mut request).await.unwrap_or_default();
                let request = String::from_utf8_lossy(&request[..read]);
                if request.starts_with("GET /redirect ") {
                    let response = format!(
                        "HTTP/1.1 302 Found\r\nLocation: http://localhost:{}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        address.port()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                    return;
                }
                if request.starts_with("GET /large-download ") {
                    let body = vec![b'x'; 8 * 1024];
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=large.bin\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                    let _ = socket.shutdown().await;
                    return;
                }
                if request.starts_with("GET /frame ") {
                    let body = concat!(
                        "<html><head><title>Frame fixture</title></head><body>",
                        "<main id='frame-state'>Frame ready</main><div id='frame-host'></div>",
                        "<script>const r=document.querySelector('#frame-host').attachShadow({mode:'open'});",
                        "r.innerHTML=\"<button id='frame-action'>Frame shadow action</button>\";",
                        "r.querySelector('button').onclick=()=>document.querySelector('#frame-state').textContent='Frame shadow clicked';</script>",
                        "</body></html>"
                    );
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(), body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                    return;
                }
                if request.starts_with("GET /popup ") {
                    let body = "<html><head><title>Owned popup</title></head><body><main>Popup ready</main></body></html>";
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(), body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                    return;
                }
                if request.starts_with("GET /complex ") {
                    let body = format!(
                        concat!(
                            "<html><head><title>Complex CDP fixture</title></head><body>",
                            "<main id='state'>Complex ready</main>",
                            "<select id='plan' onchange=\"document.querySelector('#state').textContent='selected:'+this.value\"><option value='basic'>Basic</option><option value='pro'>Professional</option></select>",
                            "<button id='hover' onmouseenter=\"document.querySelector('#state').textContent='hovered'\">Hover action</button>",
                            "<button id='popup' onclick=\"window.open('/popup','_blank')\">Open popup</button>",
                            "<button id='dialog' onclick=\"alert('fixture dialog');document.querySelector('#state').textContent='dialog handled'\">Show dialog</button>",
                            "<div id='shadow-host'></div>",
                            "<div id='scroller' tabindex='0' style='height:80px;overflow:auto' onscroll=\"document.querySelector('#scroll-state').textContent='scrolled'\"><div style='height:700px'></div><button id='offscreen'>Offscreen action</button></div><output id='scroll-state'>not scrolled</output>",
                            "<iframe src='/frame'></iframe><iframe src='http://localhost:{}/frame'></iframe>",
                            "<script>const a=document.querySelector('#shadow-host').attachShadow({{mode:'open'}});a.innerHTML=\"<section id='nested'></section>\";const b=a.querySelector('#nested').attachShadow({{mode:'open'}});b.innerHTML=\"<button>Nested shadow action</button>\";b.querySelector('button').onclick=()=>document.querySelector('#state').textContent='shadow clicked';</script>",
                            "</body></html>"
                        ),
                        address.port()
                    );
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(), body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                    return;
                }
                let body = concat!(
                    "<html><head><title>OpenTopia browser test</title></head>",
                    "<body><h1>Browser runtime works</h1>",
                    "<button id='press' onclick=\"this.textContent='Pressed'\">Press</button>",
                    "<input id='field' /></body></html>"
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });

    let mut config = BrowserRuntimeConfig::default();
    let data_root = std::env::temp_dir().join(format!("opentopia-browser-test-{}", Uuid::new_v4()));
    config.data_root = data_root.clone();
    config.startup_timeout = Duration::from_secs(20);
    config.max_download_bytes = 1024;
    let runtime = LocalBrowserRuntime::new(config);
    let session = BrowserSessionId::new();
    let spec = BrowserSessionSpec {
        session_id: session,
        profile_id: BrowserProfileId::new("runtime-smoke").unwrap(),
        profile_persistence: BrowserProfilePersistence::Ephemeral,
    };
    let ephemeral_root = data_root.join("ephemeral").join("runtime-smoke");
    let info = runtime.create_session(spec).await.unwrap();
    assert_eq!(
        info.profile_persistence,
        BrowserProfilePersistence::Ephemeral
    );
    let conflict = runtime
        .create_session(BrowserSessionSpec::from(session))
        .await
        .unwrap_err();
    assert!(matches!(
        conflict,
        BrowserError::SessionProfileConflict { session: conflicted } if conflicted == session
    ));
    let url = format!("http://{address}/");

    runtime
        .grant_network_access(
            session,
            BrowserNetworkGrant::new([address.ip().to_string()]).unwrap(),
        )
        .await
        .unwrap();

    runtime
        .navigate(session, BrowserNavigateRequest::new(url))
        .await
        .unwrap();
    let observation = runtime
        .observe(session, BrowserObserveOptions::default())
        .await
        .unwrap();
    assert!(observation.text.contains("Browser runtime works"));
    let press = observation
        .nodes
        .iter()
        .find(|node| node.name == "Press")
        .expect("press button must be observable");

    let screenshot = runtime.screenshot(session).await.unwrap();
    assert!(matches!(
        screenshot.contents.first(),
        Some(BrowserContent::Image { bytes, .. }) if bytes.starts_with(b"\x89PNG")
    ));

    let click_receipt = runtime
        .perform(
            session,
            observation.observation_id,
            press.node_ref,
            BrowserAction::Click,
        )
        .await
        .unwrap();
    assert!(click_receipt.verification.page_changed);
    assert!(click_receipt.verification.text_changed);

    assert!(matches!(
        runtime
            .download(
                session,
                BrowserDownloadRequest {
                    url: format!("http://{address}/large-download"),
                    expected_filename: Some("large.bin".to_string()),
                    timeout: Some(Duration::from_secs(5)),
                },
            )
            .await,
        Err(BrowserError::DownloadTooLarge { maximum: 1024 })
    ));

    assert!(matches!(
        runtime
            .perform(
                session,
                observation.observation_id,
                press.node_ref,
                BrowserAction::Click,
            )
            .await,
        Err(BrowserError::StaleObservation { .. })
    ));

    let refreshed = runtime
        .observe(session, BrowserObserveOptions::default())
        .await
        .unwrap();
    let field = refreshed
        .nodes
        .iter()
        .find(|node| node.tag_name == "input")
        .expect("input must be observable");
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
        .await
        .unwrap();

    assert!(matches!(
        runtime
            .navigate(
                session,
                BrowserNavigateRequest::new(format!("http://{address}/redirect")),
            )
            .await,
        Err(BrowserError::NetworkBlocked { ref host }) if host == "localhost"
    ));

    runtime
        .grant_network_access(session, BrowserNetworkGrant::new(["localhost"]).unwrap())
        .await
        .unwrap();
    runtime
        .navigate(
            session,
            BrowserNavigateRequest::new(format!("http://{address}/complex")),
        )
        .await
        .unwrap();
    let complex = runtime
        .observe(session, BrowserObserveOptions::default())
        .await
        .unwrap();
    assert!(complex.frames.len() >= 3);
    assert!(!complex.accessibility_tree.is_empty());
    assert!(complex
        .nodes
        .iter()
        .any(|node| node.name == "Nested shadow action"));
    assert!(
        complex
            .nodes
            .iter()
            .any(|node| node.name == "Frame shadow action"),
        "captured frames: {:?}; nodes: {:?}",
        complex.frames,
        complex
            .nodes
            .iter()
            .map(|node| (&node.name, &node.frame_ref))
            .collect::<Vec<_>>()
    );
    let initial_target = complex
        .targets
        .iter()
        .find(|target| target.active)
        .unwrap()
        .target_ref
        .clone();
    let select = complex
        .nodes
        .iter()
        .find(|node| node.tag_name == "select")
        .unwrap();
    runtime
        .perform(
            session,
            complex.observation_id,
            select.node_ref,
            BrowserAction::Select {
                value: "pro".to_string(),
            },
        )
        .await
        .unwrap();
    let complex = runtime
        .observe(session, BrowserObserveOptions::default())
        .await
        .unwrap();
    assert!(complex.text.contains("selected:pro"));
    let popup = complex
        .nodes
        .iter()
        .find(|node| node.name == "Open popup")
        .unwrap();
    runtime
        .perform(
            session,
            complex.observation_id,
            popup.node_ref,
            BrowserAction::Click,
        )
        .await
        .unwrap();
    let popup_observation = runtime
        .observe(session, BrowserObserveOptions::default())
        .await
        .unwrap();
    assert!(popup_observation.targets.len() >= 2);
    assert!(popup_observation.text.contains("Popup ready"));
    runtime
        .switch_target(session, initial_target)
        .await
        .unwrap();
    let complex = runtime
        .observe(session, BrowserObserveOptions::default())
        .await
        .unwrap();
    let dialog = complex
        .nodes
        .iter()
        .find(|node| node.name == "Show dialog")
        .unwrap();
    runtime
        .perform(
            session,
            complex.observation_id,
            dialog.node_ref,
            BrowserAction::Click,
        )
        .await
        .unwrap();
    let after_dialog = runtime
        .observe(session, BrowserObserveOptions::default())
        .await
        .unwrap();
    assert!(after_dialog
        .dialogs
        .iter()
        .any(|dialog| { dialog.message == "fixture dialog" && dialog.handled }));

    runtime.close_session(session).await.unwrap();
    assert!(!ephemeral_root.exists());
    server.abort();
}
