use super::super::apply_provider_auth;
use super::OpenAiCompatibleProvider;
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

// Compatibility setup is an infrequent action, while relay endpoints often
// enforce very small per-key upstream concurrency limits. Serialize probes so
// the feature matrix cannot turn into a burst that trips those limits.
const PROBE_CONCURRENCY_LIMIT: usize = 1;
const PROBE_RATE_LIMIT_RETRIES: usize = 2;
const PROBE_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

/// One connection test performs several independent feature probes. They share
/// this client so the test behaves like a polite session instead of an
/// unbounded burst against one API key.
#[derive(Clone)]
pub(in crate::provider) struct OpenAiProbeClient {
    provider: OpenAiCompatibleProvider,
    concurrency: Arc<Semaphore>,
    cooldown_until: Arc<Mutex<Instant>>,
}

pub(in crate::provider) struct OpenAiProbeResponse {
    response: reqwest::Response,
    permit: OwnedSemaphorePermit,
}

impl OpenAiProbeResponse {
    pub(in crate::provider) fn into_parts(self) -> (reqwest::Response, OwnedSemaphorePermit) {
        (self.response, self.permit)
    }
}

impl OpenAiProbeClient {
    pub(in crate::provider) fn new(provider: &OpenAiCompatibleProvider) -> Self {
        Self {
            provider: provider.clone(),
            concurrency: Arc::new(Semaphore::new(PROBE_CONCURRENCY_LIMIT)),
            cooldown_until: Arc::new(Mutex::new(Instant::now())),
        }
    }

    pub(in crate::provider) async fn send(
        &self,
        path: &str,
        payload: &Value,
    ) -> Result<OpenAiProbeResponse, String> {
        let url = format!("{}{}", self.provider.base_url.trim_end_matches('/'), path);
        let mut rate_limit_retry = 0;
        loop {
            self.wait_for_cooldown().await;
            let permit = Arc::clone(&self.concurrency)
                .acquire_owned()
                .await
                .map_err(|_| "compatibility probe concurrency gate closed".to_string())?;
            let response = tokio::time::timeout(
                PROBE_REQUEST_TIMEOUT,
                apply_provider_auth(
                    self.provider.client.post(&url),
                    self.provider.auth,
                    &self.provider.api_key,
                )
                .header(CONTENT_TYPE, "application/json")
                .json(payload)
                .send(),
            )
            .await
            .map_err(|_| "request timed out after 20 seconds".to_string())?
            .map_err(|error| error.to_string())?;

            if response.status() != reqwest::StatusCode::TOO_MANY_REQUESTS
                || rate_limit_retry >= PROBE_RATE_LIMIT_RETRIES
            {
                return Ok(OpenAiProbeResponse { response, permit });
            }

            rate_limit_retry += 1;
            let delay = retry_after_delay(&response, rate_limit_retry);
            self.extend_cooldown(delay).await;
            // Consume the body before reusing the connection. Diagnostics for
            // the final attempt remain available to the caller.
            let _ = response.text().await;
            drop(permit);
        }
    }

    async fn wait_for_cooldown(&self) {
        loop {
            let deadline = *self.cooldown_until.lock().await;
            let now = Instant::now();
            if deadline <= now {
                return;
            }
            tokio::time::sleep_until(deadline).await;
        }
    }

    async fn extend_cooldown(&self, delay: Duration) {
        let target = Instant::now() + delay;
        let mut cooldown_until = self.cooldown_until.lock().await;
        if target > *cooldown_until {
            *cooldown_until = target;
        }
    }
}

fn retry_after_delay(response: &reqwest::Response, retry_index: usize) -> Duration {
    response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .map(|delay| delay.min(PROBE_MAX_RETRY_AFTER))
        .unwrap_or_else(|| Duration::from_secs(1_u64 << retry_index.min(4)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn serializes_concurrently_scheduled_probes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let active_requests = Arc::new(AtomicUsize::new(0));
        let peak_requests = Arc::new(AtomicUsize::new(0));
        let server_active_requests = Arc::clone(&active_requests);
        let server_peak_requests = Arc::clone(&peak_requests);
        let server = tokio::spawn(async move {
            let mut handlers = Vec::new();
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let active_requests = Arc::clone(&server_active_requests);
                let peak_requests = Arc::clone(&server_peak_requests);
                handlers.push(tokio::spawn(async move {
                    let mut buffer = [0_u8; 2_048];
                    let _ = socket.read(&mut buffer).await.unwrap();
                    let active = active_requests.fetch_add(1, Ordering::SeqCst) + 1;
                    peak_requests.fetch_max(active, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                        )
                        .await
                        .unwrap();
                    active_requests.fetch_sub(1, Ordering::SeqCst);
                }));
            }
            for handler in handlers {
                handler.await.unwrap();
            }
            server_peak_requests.load(Ordering::SeqCst)
        });

        let provider = OpenAiCompatibleProvider::new(
            format!("http://{address}/v1"),
            "test-key",
            "probe-model",
        );
        let client = OpenAiProbeClient::new(&provider);
        let send_probe = |client: OpenAiProbeClient| async move {
            let payload = serde_json::json!({ "model": "probe-model" });
            let response = client.send("/chat/completions", &payload).await.unwrap();
            let (response, permit) = response.into_parts();
            let _ = response.text().await.unwrap();
            drop(permit);
        };
        tokio::join!(
            send_probe(client.clone()),
            send_probe(client.clone()),
            send_probe(client),
        );

        assert_eq!(server.await.unwrap(), 1);
    }
}
