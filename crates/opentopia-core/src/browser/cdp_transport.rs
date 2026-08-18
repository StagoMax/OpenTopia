//! Chrome DevTools Protocol transport and the attached Chrome bridge actor.

use super::io_support::{network_policy_allows_url, network_request_host, normalize_network_host};
use super::{
    BrowserError, BrowserNetworkGrant, BrowserSessionId, MAX_CHROME_BRIDGE_RESPONSE_BYTES,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::{header::ORIGIN, HeaderValue},
        Message as WebSocketMessage,
    },
    MaybeTlsStream, WebSocketStream,
};

type CdpSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

enum CdpCommand {
    Execute {
        client_id: u64,
        method: String,
        params: Value,
        session_id: Option<String>,
        response: oneshot::Sender<Result<Value, BrowserError>>,
    },
    Cancel {
        client_id: u64,
    },
}

#[derive(Debug, Clone)]
pub(super) struct CdpEvent {
    pub(super) method: String,
    pub(super) params: Value,
    pub(super) session_id: Option<String>,
}

pub(super) struct CdpPage {
    commands: mpsc::Sender<CdpCommand>,
    events: broadcast::Receiver<CdpEvent>,
    buffered_events: Vec<CdpEvent>,
    command_timeout: Duration,
    pub(super) session_id: Option<String>,
    pub(super) target_id: Option<String>,
    next_client_id: Arc<AtomicU64>,
    network_policy: Arc<StdRwLock<Option<HashSet<String>>>>,
    connected: Arc<AtomicBool>,
}

impl CdpPage {
    pub(super) async fn connect(
        websocket_url: &str,
        command_timeout: Duration,
    ) -> Result<Self, BrowserError> {
        let mut request = websocket_url
            .into_client_request()
            .map_err(|error| BrowserError::Protocol(error.to_string()))?;
        request
            .headers_mut()
            .insert(ORIGIN, HeaderValue::from_static("http://localhost"));
        let (socket, _) = tokio::time::timeout(command_timeout, connect_async(request))
            .await
            .map_err(|_| BrowserError::Timeout("connecting to the local browser".to_string()))?
            .map_err(|error| BrowserError::Protocol(error.to_string()))?;
        let (write, read) = socket.split();
        let (commands, command_rx) = mpsc::channel(64);
        let (event_tx, events) = broadcast::channel(512);
        let network_policy = Arc::new(StdRwLock::new(None));
        let connected = Arc::new(AtomicBool::new(true));
        tokio::spawn(run_cdp_connection(
            write,
            read,
            command_rx,
            event_tx,
            network_policy.clone(),
            connected.clone(),
        ));
        Ok(Self {
            commands,
            events,
            buffered_events: Vec::new(),
            command_timeout,
            session_id: None,
            target_id: None,
            next_client_id: Arc::new(AtomicU64::new(0)),
            network_policy,
            connected,
        })
    }

    pub(super) async fn connect_chrome_bridge(
        bridge_url: &str,
        bridge_token: &str,
        browser_session_id: BrowserSessionId,
        command_timeout: Duration,
    ) -> Result<Self, BrowserError> {
        let base_url = validate_chrome_bridge_url(bridge_url)?;
        let authorization =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", bridge_token.trim()))
                .map_err(|_| {
                    BrowserError::BrokerConfiguration("Chrome bridge token is invalid".to_string())
                })?;
        if bridge_token.trim().is_empty() {
            return Err(BrowserError::BrokerConfiguration(
                "Chrome bridge token is empty".to_string(),
            ));
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(command_timeout.min(Duration::from_secs(5)))
            .build()?;
        let session_url = base_url
            .join(&format!("v1/backend/sessions/{browser_session_id}"))
            .map_err(|_| BrowserError::Protocol("Invalid Chrome session endpoint".to_string()))?;
        let response = tokio::time::timeout(
            command_timeout,
            client
                .get(session_url)
                .header(reqwest::header::AUTHORIZATION, authorization.clone())
                .send(),
        )
        .await
        .map_err(|_| BrowserError::Timeout("Chrome session attach".to_string()))?
        .map_err(map_chrome_bridge_transport_error)?;
        let status = response.status();
        let metadata = read_chrome_bridge_json(response).await?;
        if !status.is_success() {
            return Err(chrome_bridge_rejected(status.as_u16(), &metadata));
        }
        let target_id = metadata
            .get("targetId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                BrowserError::Protocol("Chrome bridge returned no target ID".to_string())
            })?
            .to_string();

        let (commands, command_rx) = mpsc::channel(64);
        let (event_tx, events) = broadcast::channel(512);
        let network_policy = Arc::new(StdRwLock::new(None));
        let connected = Arc::new(AtomicBool::new(true));
        tokio::spawn(run_chrome_bridge_commands(
            client.clone(),
            base_url.clone(),
            authorization.clone(),
            browser_session_id,
            command_timeout,
            command_rx,
            connected.clone(),
        ));
        tokio::spawn(run_chrome_bridge_events(
            client,
            base_url,
            authorization,
            browser_session_id,
            event_tx,
            connected.clone(),
        ));
        Ok(Self {
            commands,
            events,
            buffered_events: Vec::new(),
            command_timeout,
            session_id: Some("root".to_string()),
            target_id: Some(target_id),
            next_client_id: Arc::new(AtomicU64::new(0)),
            network_policy,
            connected,
        })
    }

    pub(super) fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    pub(super) fn grant_network_access(
        &self,
        grant: BrowserNetworkGrant,
    ) -> Result<(), BrowserError> {
        let mut policy = self.network_policy.write().map_err(|_| {
            BrowserError::Protocol("Browser network policy lock is poisoned".into())
        })?;
        let allowed_hosts = policy.get_or_insert_with(HashSet::new);
        for host in grant.allowed_hosts {
            allowed_hosts.insert(normalize_network_host(&host)?);
        }
        Ok(())
    }

    pub(super) async fn create_and_attach_target(&mut self) -> Result<(), BrowserError> {
        let target = self
            .command("Target.createTarget", json!({ "url": "about:blank" }))
            .await?;
        let target_id = target
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                BrowserError::Protocol("Target.createTarget returned no target ID".to_string())
            })?;
        let attached = self
            .command(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await?;
        self.target_id = Some(target_id.to_string());
        self.session_id = Some(
            attached
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    BrowserError::Protocol(
                        "Target.attachToTarget returned no session ID".to_string(),
                    )
                })?
                .to_string(),
        );
        self.discard_events();
        Ok(())
    }

    pub(super) async fn initialize_page_domains(&mut self) -> Result<(), BrowserError> {
        let session_id = self.session_id.clone().ok_or_else(|| {
            BrowserError::Protocol("No active CDP page session is attached".to_string())
        })?;
        self.initialize_page_domains_for_session(&session_id).await
    }

    pub(super) async fn initialize_external_page_domains(&mut self) -> Result<(), BrowserError> {
        let session_id = self.session_id.clone().ok_or_else(|| {
            BrowserError::Protocol("No active Chrome page session is attached".to_string())
        })?;
        self.initialize_external_page_domains_for_session(&session_id)
            .await
    }

    pub(super) async fn initialize_external_page_domains_for_session(
        &self,
        session_id: &str,
    ) -> Result<(), BrowserError> {
        let session_id = Some(session_id.to_string());
        self.command_for_session("Page.enable", json!({}), session_id.clone())
            .await?;
        self.command_for_session("Runtime.enable", json!({}), session_id.clone())
            .await?;
        self.command_for_session("Network.enable", json!({}), session_id.clone())
            .await?;
        self.command_for_session(
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
            session_id,
        )
        .await
        .map(|_| ())
    }

    pub(super) async fn initialize_page_domains_for_session(
        &self,
        session_id: &str,
    ) -> Result<(), BrowserError> {
        let session_id = Some(session_id.to_string());
        self.command_for_session("Page.enable", json!({}), session_id.clone())
            .await
            .map_err(|error| {
                BrowserError::Protocol(format!("enabling the Page domain: {error}"))
            })?;
        self.command_for_session("Runtime.enable", json!({}), session_id.clone())
            .await
            .map_err(|error| {
                BrowserError::Protocol(format!("enabling the Runtime domain: {error}"))
            })?;
        self.command_for_session("Network.enable", json!({}), session_id.clone())
            .await
            .map_err(|error| {
                BrowserError::Protocol(format!("enabling the Network domain: {error}"))
            })?;
        self.command_for_session(
            "Fetch.enable",
            json!({ "patterns": [{ "urlPattern": "http://*/*", "requestStage": "Request" }, { "urlPattern": "https://*/*", "requestStage": "Request" }] }),
            session_id.clone(),
        )
        .await
        .map_err(|error| {
            BrowserError::Protocol(format!("enabling request interception: {error}"))
        })?;
        self.command_for_session(
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
            session_id,
        )
        .await
        .map(|_| ())
    }

    pub(super) async fn enable_target_discovery(&self) -> Result<(), BrowserError> {
        self.root_command("Target.setDiscoverTargets", json!({ "discover": true }))
            .await?;
        self.enable_target_auto_attach().await
    }

    pub(super) async fn enable_target_auto_attach(&self) -> Result<(), BrowserError> {
        self.root_command(
            "Target.setAutoAttach",
            json!({
                "autoAttach": true,
                "waitForDebuggerOnStart": false,
                "flatten": true,
                "filter": [
                    { "type": "page", "exclude": false },
                    { "exclude": true }
                ]
            }),
        )
        .await
        .map(|_| ())
    }

    pub(super) fn activate(&mut self, target_id: String, session_id: String) {
        self.target_id = Some(target_id);
        self.session_id = Some(session_id);
    }

    pub(super) async fn configure_downloads(
        &self,
        download_dir: &Path,
    ) -> Result<(), BrowserError> {
        self.root_command(
            "Browser.setDownloadBehavior",
            json!({
                "behavior": "allow",
                "downloadPath": download_dir,
                "eventsEnabled": true,
            }),
        )
        .await
        .map(|_| ())
    }

    pub(super) async fn command(&self, method: &str, params: Value) -> Result<Value, BrowserError> {
        self.command_for_session(method, params, self.session_id.clone())
            .await
    }

    pub(super) async fn root_command(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, BrowserError> {
        self.command_for_session(method, params, None).await
    }

    pub(super) async fn command_for_session(
        &self,
        method: &str,
        params: Value,
        session_id: Option<String>,
    ) -> Result<Value, BrowserError> {
        let (response, receiver) = oneshot::channel();
        let client_id = self
            .next_client_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        self.commands
            .send(CdpCommand::Execute {
                client_id,
                method: method.to_string(),
                params,
                session_id,
                response,
            })
            .await
            .map_err(|_| {
                BrowserError::Disconnected("DevTools command channel closed".to_string())
            })?;
        match tokio::time::timeout(self.command_timeout, receiver).await {
            Ok(response) => response.map_err(|_| {
                BrowserError::Disconnected("DevTools response channel closed".to_string())
            })?,
            Err(_) => {
                let _ = self.commands.try_send(CdpCommand::Cancel { client_id });
                Err(BrowserError::Timeout(method.to_string()))
            }
        }
    }

    pub(super) fn discard_events(&mut self) {
        self.buffered_events.clear();
        loop {
            match self.events.try_recv() {
                Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(broadcast::error::TryRecvError::Empty)
                | Err(broadcast::error::TryRecvError::Closed) => break,
            }
        }
    }

    pub(super) fn try_next_event(&mut self) -> Result<Option<CdpEvent>, BrowserError> {
        while let Some(event) = self.buffered_events.pop() {
            if self.event_belongs_to_session(&event) {
                return Ok(Some(event));
            }
        }
        loop {
            match self.events.try_recv() {
                Ok(event) if self.event_belongs_to_session(&event) => return Ok(Some(event)),
                Ok(_) => continue,
                Err(broadcast::error::TryRecvError::Empty) => return Ok(None),
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(broadcast::error::TryRecvError::Closed) => {
                    return Err(BrowserError::Disconnected(
                        "Browser closed the DevTools event stream".to_string(),
                    ));
                }
            }
        }
    }

    pub(super) async fn next_event(&mut self) -> Result<Option<CdpEvent>, BrowserError> {
        loop {
            match self.events.recv().await {
                Ok(event) if self.event_belongs_to_session(&event) => return Ok(Some(event)),
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(BrowserError::Disconnected(
                        "Browser closed the DevTools event stream".to_string(),
                    ));
                }
            }
        }
    }

    pub(super) fn push_event(&mut self, event: CdpEvent) {
        self.buffered_events.push(event);
    }

    pub(super) fn event_belongs_to_session(&self, event: &CdpEvent) -> bool {
        event.session_id.as_deref() == self.session_id.as_deref()
            || (event.session_id.is_none()
                && (event.method.starts_with("Browser.download")
                    || event.method.starts_with("Target.")))
    }
}

async fn run_cdp_connection(
    mut write: futures_util::stream::SplitSink<CdpSocket, WebSocketMessage>,
    mut read: futures_util::stream::SplitStream<CdpSocket>,
    mut commands: mpsc::Receiver<CdpCommand>,
    events: broadcast::Sender<CdpEvent>,
    network_policy: Arc<StdRwLock<Option<HashSet<String>>>>,
    connected: Arc<AtomicBool>,
) {
    let mut next_id = 0_u64;
    let mut pending =
        HashMap::<u64, (u64, String, oneshot::Sender<Result<Value, BrowserError>>)>::new();
    let disconnect_reason = loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    break "DevTools command channel closed".to_string();
                };
                pending.retain(|_, (_, _, response)| !response.is_closed());
                match command {
                    CdpCommand::Cancel { client_id } => {
                        pending.retain(|_, (pending_client_id, _, _)| *pending_client_id != client_id);
                    }
                    CdpCommand::Execute { client_id, method, params, session_id, response } => {
                        next_id = next_id.saturating_add(1);
                        let id = next_id;
                        let mut message = json!({ "id": id, "method": method, "params": params });
                        if let Some(session_id) = session_id.as_ref() {
                            message["sessionId"] = Value::String(session_id.clone());
                        }
                        if let Err(error) = write.send(WebSocketMessage::Text(message.to_string())).await {
                            let reason = error.to_string();
                            let _ = response.send(Err(BrowserError::Disconnected(reason.clone())));
                            break reason;
                        }
                        pending.insert(id, (client_id, method, response));
                    }
                }
            }
            incoming = read.next() => {
                let Some(incoming) = incoming else {
                    break "Browser closed the DevTools connection".to_string();
                };
                match incoming {
                    Ok(WebSocketMessage::Text(text)) => {
                        let message = match serde_json::from_str::<Value>(&text) {
                            Ok(message) => message,
                            Err(error) => break format!("Invalid DevTools message: {error}"),
                        };
                        if let Some(id) = message.get("id").and_then(Value::as_u64) {
                            if let Some((_, method, response)) = pending.remove(&id) {
                                let result = if let Some(error) = message.get("error") {
                                    Err(BrowserError::Cdp {
                                        method,
                                        message: error
                                            .get("message")
                                            .and_then(Value::as_str)
                                            .unwrap_or("Unknown DevTools error")
                                            .to_string(),
                                    })
                                } else {
                                    Ok(message.get("result").cloned().unwrap_or(Value::Null))
                                };
                                let _ = response.send(result);
                            }
                        } else if let Some(method) = message.get("method").and_then(Value::as_str) {
                            if method == "Target.attachedToTarget" {
                                if let Some(attached_session_id) = message
                                    .pointer("/params/sessionId")
                                    .and_then(Value::as_str)
                                {
                                    next_id = next_id.saturating_add(1);
                                    let resume = json!({
                                        "id": next_id,
                                        "method": "Runtime.runIfWaitingForDebugger",
                                        "params": {},
                                        "sessionId": attached_session_id,
                                    });
                                    if let Err(error) = write
                                        .send(WebSocketMessage::Text(resume.to_string()))
                                        .await
                                    {
                                        break error.to_string();
                                    }
                                }
                            }
                            if method == "Page.javascriptDialogOpening" {
                                next_id = next_id.saturating_add(1);
                                let mut dismissal = json!({
                                    "id": next_id,
                                    "method": "Page.handleJavaScriptDialog",
                                    "params": { "accept": false },
                                });
                                if let Some(session_id) = message.get("sessionId").and_then(Value::as_str) {
                                    dismissal["sessionId"] = Value::String(session_id.to_string());
                                }
                                if let Err(error) = write
                                    .send(WebSocketMessage::Text(dismissal.to_string()))
                                    .await
                                {
                                    break error.to_string();
                                }
                            }
                            if method == "Fetch.requestPaused" {
                                let params = message.get("params").cloned().unwrap_or(Value::Null);
                                let request_id = params
                                    .get("requestId")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                let request_url = params
                                    .pointer("/request/url")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                let allowed = network_policy_allows_url(&network_policy, request_url);
                                next_id = next_id.saturating_add(1);
                                let interception_method = if allowed {
                                    "Fetch.continueRequest"
                                } else {
                                    "Fetch.failRequest"
                                };
                                let interception_params = if allowed {
                                    json!({ "requestId": request_id })
                                } else {
                                    json!({ "requestId": request_id, "errorReason": "BlockedByClient" })
                                };
                                let mut interception = json!({
                                    "id": next_id,
                                    "method": interception_method,
                                    "params": interception_params,
                                });
                                if let Some(session_id) = message.get("sessionId").and_then(Value::as_str) {
                                    interception["sessionId"] = Value::String(session_id.to_string());
                                }
                                if let Err(error) = write
                                    .send(WebSocketMessage::Text(interception.to_string()))
                                    .await
                                {
                                    break error.to_string();
                                }
                                if !allowed {
                                    let _ = events.send(CdpEvent {
                                        method: "OpenTopia.networkRequestBlocked".to_string(),
                                        params: json!({
                                            "url": request_url,
                                            "host": network_request_host(request_url),
                                            "resourceType": params.get("resourceType").cloned(),
                                        }),
                                        session_id: message
                                            .get("sessionId")
                                            .and_then(Value::as_str)
                                            .map(str::to_string),
                                    });
                                }
                                continue;
                            }
                            let _ = events.send(CdpEvent {
                                method: method.to_string(),
                                params: message.get("params").cloned().unwrap_or(Value::Null),
                                session_id: message
                                    .get("sessionId")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                            });
                        }
                    }
                    Ok(WebSocketMessage::Ping(payload)) => {
                        if let Err(error) = write.send(WebSocketMessage::Pong(payload)).await {
                            break error.to_string();
                        }
                    }
                    Ok(WebSocketMessage::Close(_)) => {
                        break "Browser closed the DevTools connection".to_string();
                    }
                    Ok(WebSocketMessage::Binary(_))
                    | Ok(WebSocketMessage::Pong(_))
                    | Ok(WebSocketMessage::Frame(_)) => {}
                    Err(error) => break error.to_string(),
                }
            }
        }
    };

    connected.store(false, Ordering::Release);
    for (_, (_, _, response)) in pending {
        let _ = response.send(Err(BrowserError::Disconnected(disconnect_reason.clone())));
    }
}

async fn run_chrome_bridge_commands(
    client: reqwest::Client,
    base_url: reqwest::Url,
    authorization: reqwest::header::HeaderValue,
    browser_session_id: BrowserSessionId,
    command_timeout: Duration,
    mut commands: mpsc::Receiver<CdpCommand>,
    connected: Arc<AtomicBool>,
) {
    let endpoint = match base_url.join("v1/backend/command") {
        Ok(endpoint) => endpoint,
        Err(_) => {
            connected.store(false, Ordering::Release);
            return;
        }
    };
    while let Some(command) = commands.recv().await {
        match command {
            CdpCommand::Cancel { .. } => {}
            CdpCommand::Execute {
                method,
                params,
                session_id,
                response,
                ..
            } => {
                let target_session_id = session_id.unwrap_or_else(|| "root".to_string());
                let request = client
                    .post(endpoint.clone())
                    .header(reqwest::header::AUTHORIZATION, authorization.clone())
                    .json(&json!({
                        "sessionId": browser_session_id,
                        "targetSessionId": target_session_id,
                        "method": method,
                        "params": params,
                    }));
                let result = match tokio::time::timeout(command_timeout, request.send()).await {
                    Err(_) => Err(BrowserError::Timeout(method)),
                    Ok(Err(error)) => Err(map_chrome_bridge_transport_error(error)),
                    Ok(Ok(http_response)) => {
                        let status = http_response.status();
                        match read_chrome_bridge_json(http_response).await {
                            Ok(value) if status.is_success() => {
                                Ok(value.get("result").cloned().unwrap_or(Value::Null))
                            }
                            Ok(value) => Err(chrome_bridge_rejected(status.as_u16(), &value)),
                            Err(error) => Err(error),
                        }
                    }
                };
                if matches!(result, Err(BrowserError::BrokerUnavailable)) {
                    connected.store(false, Ordering::Release);
                }
                let _ = response.send(result);
            }
        }
    }
    connected.store(false, Ordering::Release);
}

async fn run_chrome_bridge_events(
    client: reqwest::Client,
    base_url: reqwest::Url,
    authorization: reqwest::header::HeaderValue,
    browser_session_id: BrowserSessionId,
    events: broadcast::Sender<CdpEvent>,
    connected: Arc<AtomicBool>,
) {
    let endpoint = match base_url.join(&format!("v1/backend/events/{browser_session_id}")) {
        Ok(endpoint) => endpoint,
        Err(_) => {
            connected.store(false, Ordering::Release);
            return;
        }
    };
    let mut after = 0_u64;
    while connected.load(Ordering::Acquire) {
        let response = client
            .get(endpoint.clone())
            .header(reqwest::header::AUTHORIZATION, authorization.clone())
            .query(&[
                ("after", after.to_string()),
                ("waitMs", "25000".to_string()),
            ])
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(_) => {
                connected.store(false, Ordering::Release);
                break;
            }
        };
        let status = response.status();
        let payload = match read_chrome_bridge_json(response).await {
            Ok(payload) if status.is_success() => payload,
            _ => {
                connected.store(false, Ordering::Release);
                break;
            }
        };
        if payload.get("attached").and_then(Value::as_bool) == Some(false) {
            connected.store(false, Ordering::Release);
            break;
        }
        for event in payload
            .get("events")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(sequence) = event.get("seq").and_then(Value::as_u64) else {
                continue;
            };
            let Some(method) = event.get("method").and_then(Value::as_str) else {
                continue;
            };
            after = after.max(sequence);
            let _ = events.send(CdpEvent {
                method: method.to_string(),
                params: event.get("params").cloned().unwrap_or(Value::Null),
                session_id: event
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }
}

pub(crate) fn validate_chrome_bridge_url(raw: &str) -> Result<reqwest::Url, BrowserError> {
    let mut url = reqwest::Url::parse(raw.trim()).map_err(|_| {
        BrowserError::BrokerConfiguration("Chrome bridge URL is invalid".to_string())
    })?;
    if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some() {
        return Err(BrowserError::BrokerConfiguration(
            "Chrome bridge URL must be credential-free HTTP".to_string(),
        ));
    }
    let loopback = url
        .host_str()
        .and_then(|host| host.parse::<IpAddr>().ok())
        .is_some_and(|address| address.is_loopback());
    if !loopback || url.query().is_some() || url.fragment().is_some() {
        return Err(BrowserError::BrokerConfiguration(
            "Chrome bridge URL must use a numeric loopback address".to_string(),
        ));
    }
    url.set_path(&format!("{}/", url.path().trim_end_matches('/')));
    Ok(url)
}

async fn read_chrome_bridge_json(response: reqwest::Response) -> Result<Value, BrowserError> {
    let content_length = response.content_length();
    if content_length.is_some_and(|length| length > MAX_CHROME_BRIDGE_RESPONSE_BYTES as u64) {
        return Err(BrowserError::BrokerResponseTooLarge {
            actual: content_length.unwrap_or_default() as usize,
            maximum: MAX_CHROME_BRIDGE_RESPONSE_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(content_length.unwrap_or_default() as usize);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(map_chrome_bridge_transport_error)?;
        let actual = bytes.len().saturating_add(chunk.len());
        if actual > MAX_CHROME_BRIDGE_RESPONSE_BYTES {
            return Err(BrowserError::BrokerResponseTooLarge {
                actual,
                maximum: MAX_CHROME_BRIDGE_RESPONSE_BYTES,
            });
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| BrowserError::Protocol("Chrome bridge returned invalid JSON".to_string()))
}

fn chrome_bridge_rejected(status: u16, body: &Value) -> BrowserError {
    let message = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("Chrome bridge rejected the request")
        .chars()
        .take(512)
        .collect();
    BrowserError::BrokerRejected { status, message }
}

fn map_chrome_bridge_transport_error(error: reqwest::Error) -> BrowserError {
    if error.is_timeout() {
        BrowserError::Timeout("Chrome bridge response".to_string())
    } else {
        BrowserError::BrokerUnavailable
    }
}
