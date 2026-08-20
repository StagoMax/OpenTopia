use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::header::RETRY_AFTER;
use reqwest::{StatusCode, Url};
use sha2::{Digest, Sha256};
use std::env;
use std::error::Error as StdError;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tracing::warn;
use uuid::Uuid;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(8);
const DEFAULT_MAX_ATTEMPTS: usize = 3;

#[derive(Debug, Clone)]
pub(crate) struct ManagedRuntimeDownloadPolicy {
    pub(crate) connect_timeout: Duration,
    pub(crate) request_timeout: Duration,
    pub(crate) initial_backoff: Duration,
    pub(crate) max_backoff: Duration,
    pub(crate) max_attempts: usize,
}

impl Default for ManagedRuntimeDownloadPolicy {
    fn default() -> Self {
        Self {
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            initial_backoff: DEFAULT_INITIAL_BACKOFF,
            max_backoff: DEFAULT_MAX_BACKOFF,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedRuntimeArtifact {
    pub(crate) url: Url,
    pub(crate) file_name: String,
    pub(crate) expected_sha256: String,
    pub(crate) download_directory: PathBuf,
    pub(crate) max_bytes: u64,
}

/// Owns a verified temporary artifact and removes it on every exit path.
/// Callers keep this guard alive through extraction and activation.
#[derive(Debug)]
pub(crate) struct VerifiedRuntimeDownload {
    path: PathBuf,
}

impl VerifiedRuntimeDownload {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for VerifiedRuntimeDownload {
    fn drop(&mut self) {
        remove_download_file(&self.path, "failed to remove managed runtime download");
    }
}

/// Owns an unverified download from the moment its path is allocated. This
/// makes cancellation safe as well as ordinary error returns: the file handle
/// is dropped before this guard and the partial artifact is then removed.
struct TemporaryRuntimeDownload {
    path: PathBuf,
    remove_on_drop: bool,
}

impl TemporaryRuntimeDownload {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            remove_on_drop: true,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn into_verified(mut self) -> VerifiedRuntimeDownload {
        self.remove_on_drop = false;
        VerifiedRuntimeDownload {
            path: std::mem::take(&mut self.path),
        }
    }
}

impl Drop for TemporaryRuntimeDownload {
    fn drop(&mut self) {
        if self.remove_on_drop {
            remove_download_file(&self.path, "failed to clean partial runtime download");
        }
    }
}

pub(crate) struct ManagedRuntimeDownloader {
    client: reqwest::Client,
    policy: ManagedRuntimeDownloadPolicy,
}

impl ManagedRuntimeDownloader {
    pub(crate) fn new(
        user_agent: &'static str,
        policy: ManagedRuntimeDownloadPolicy,
    ) -> Result<Self> {
        let builder = reqwest::Client::builder()
            .connect_timeout(policy.connect_timeout)
            .timeout(policy.request_timeout)
            .user_agent(user_agent);
        let client = configure_platform_proxy(builder).build()?;
        Ok(Self { client, policy })
    }

    pub(crate) async fn download_verified(
        &self,
        artifact: &ManagedRuntimeArtifact,
    ) -> Result<VerifiedRuntimeDownload> {
        tokio::fs::create_dir_all(&artifact.download_directory)
            .await
            .with_context(|| {
                format!(
                    "failed to create managed runtime download directory {}",
                    artifact.download_directory.display()
                )
            })?;

        let max_attempts = self.policy.max_attempts.max(1);
        for attempt in 1..=max_attempts {
            match self.download_attempt(artifact).await {
                Ok(download) => return Ok(download),
                Err(failure) if failure.retryable && attempt < max_attempts => {
                    let backoff = retry_backoff(&self.policy, attempt);
                    let delay = failure
                        .retry_after
                        .map(|value| value.max(backoff).min(self.policy.max_backoff))
                        .unwrap_or(backoff);
                    warn!(
                        url = %artifact.url,
                        attempt,
                        max_attempts,
                        delay_ms = delay.as_millis() as u64,
                        error = %failure.error,
                        "managed runtime download failed transiently; retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(failure) => {
                    return Err(failure.error).with_context(|| {
                        if failure.retryable {
                            format!(
                                "managed runtime download exhausted {max_attempts} attempts for {}",
                                artifact.url
                            )
                        } else {
                            format!("managed runtime download failed for {}", artifact.url)
                        }
                    });
                }
            }
        }
        unreachable!("managed runtime downloader always returns from its bounded attempt loop")
    }

    async fn download_attempt(
        &self,
        artifact: &ManagedRuntimeArtifact,
    ) -> std::result::Result<VerifiedRuntimeDownload, AttemptFailure> {
        let response = self
            .client
            .get(artifact.url.clone())
            .send()
            .await
            .map_err(|error| AttemptFailure::transport(error, "request failed"))?;
        let status = response.status();
        if !status.is_success() {
            let retry_after = retry_after_delay(response.headers());
            return Err(AttemptFailure {
                error: anyhow::anyhow!("download returned HTTP {status}"),
                retryable: retryable_status(status),
                retry_after,
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > artifact.max_bytes)
        {
            return Err(AttemptFailure::terminal(anyhow::anyhow!(
                "download exceeds the configured {} byte limit",
                artifact.max_bytes
            )));
        }

        let temporary = TemporaryRuntimeDownload::new(artifact.download_directory.join(format!(
            ".{}-{}.download",
            artifact.file_name,
            Uuid::new_v4()
        )));
        let mut output = tokio::fs::File::create(temporary.path())
            .await
            .map_err(|error| AttemptFailure::terminal(error.into()))?;
        let mut downloaded = 0_u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    drop(output);
                    return Err(AttemptFailure::transport(
                        error,
                        "response body failed while downloading",
                    ));
                }
            };
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            if downloaded > artifact.max_bytes {
                drop(output);
                return Err(AttemptFailure::terminal(anyhow::anyhow!(
                    "download exceeds the configured {} byte limit",
                    artifact.max_bytes
                )));
            }
            if let Err(error) = output.write_all(&chunk).await {
                drop(output);
                return Err(AttemptFailure::terminal(error.into()));
            }
        }
        if let Err(error) = output.flush().await {
            drop(output);
            return Err(AttemptFailure::terminal(error.into()));
        }
        drop(output);

        if let Err(error) = verify_file_sha256(temporary.path(), &artifact.expected_sha256).await {
            return Err(AttemptFailure::terminal(error));
        }
        Ok(temporary.into_verified())
    }
}

struct AttemptFailure {
    error: anyhow::Error,
    retryable: bool,
    retry_after: Option<Duration>,
}

impl AttemptFailure {
    fn terminal(error: anyhow::Error) -> Self {
        Self {
            error,
            retryable: false,
            retry_after: None,
        }
    }

    fn transport(error: reqwest::Error, context: &'static str) -> Self {
        let retryable = retryable_transport_error(&error);
        Self {
            error: anyhow::Error::new(error).context(context),
            retryable,
            retry_after: None,
        }
    }
}

fn retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn retryable_transport_error(error: &reqwest::Error) -> bool {
    if error.is_connect() || error.is_timeout() {
        return true;
    }
    let mut source = error.source();
    while let Some(current) = source {
        if let Some(io_error) = current.downcast_ref::<std::io::Error>() {
            if matches!(
                io_error.kind(),
                std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::UnexpectedEof
            ) {
                return true;
            }
        }
        source = current.source();
    }
    false
}

fn retry_after_delay(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn retry_backoff(policy: &ManagedRuntimeDownloadPolicy, failed_attempt: usize) -> Duration {
    let exponent = failed_attempt.saturating_sub(1).min(31) as u32;
    let multiplier = 1_u32 << exponent;
    policy
        .initial_backoff
        .checked_mul(multiplier)
        .unwrap_or(policy.max_backoff)
        .min(policy.max_backoff)
}

fn remove_download_file(path: &Path, message: &'static str) {
    if let Err(error) = fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            warn!(
                path = %path.display(),
                %error,
                cleanup_context = message,
                "managed runtime download cleanup failed"
            );
        }
    }
}

pub(crate) async fn file_sha256(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<String> {
        let mut file = fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 128 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect())
    })
    .await
    .context("managed runtime hash task failed")?
}

pub(crate) async fn verify_file_sha256(path: &Path, expected: &str) -> Result<()> {
    let actual = file_sha256(path).await?;
    anyhow::ensure!(
        actual.eq_ignore_ascii_case(expected),
        "managed runtime SHA-256 mismatch: expected {expected}, got {actual}"
    );
    Ok(())
}

pub(crate) fn managed_runtime_root() -> PathBuf {
    if let Some(root) = env::var_os("OPENTOPIA_RUNTIME_HOME") {
        return PathBuf::from(root);
    }
    if cfg!(windows) {
        return env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join("OpenTopia")
            .join("runtimes");
    }
    env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(env::temp_dir)
        .join("opentopia")
        .join("runtimes")
}

#[cfg(not(windows))]
fn configure_platform_proxy(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    builder
}

#[cfg(windows)]
fn configure_platform_proxy(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    if proxy_environment_is_configured() {
        return builder;
    }
    let Some((server, bypass)) = windows_internet_proxy() else {
        return builder;
    };
    let Ok(mut proxy) = reqwest::Proxy::all(server) else {
        return builder;
    };
    if let Some(bypass) = bypass.as_deref().and_then(reqwest::NoProxy::from_string) {
        proxy = proxy.no_proxy(Some(bypass));
    }
    builder.proxy(proxy)
}

#[cfg(windows)]
fn proxy_environment_is_configured() -> bool {
    [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ]
    .iter()
    .any(|name| env::var_os(name).is_some_and(|value| !value.is_empty()))
}

#[cfg(windows)]
fn windows_internet_proxy() -> Option<(String, Option<String>)> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let settings = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        .ok()?;
    let enabled = settings.get_value::<u32, _>("ProxyEnable").ok()?;
    if enabled == 0 {
        return None;
    }
    let configured = settings.get_value::<String, _>("ProxyServer").ok()?;
    let server = parse_windows_proxy_server(&configured)?;
    let bypass = settings
        .get_value::<String, _>("ProxyOverride")
        .ok()
        .and_then(|value| parse_windows_proxy_bypass(&value));
    Some((server, bypass))
}

#[cfg(windows)]
fn parse_windows_proxy_server(configured: &str) -> Option<String> {
    let configured = configured.trim();
    if configured.is_empty() {
        return None;
    }
    let selected = if configured.contains('=') {
        let entries = configured
            .split(';')
            .filter_map(|entry| entry.split_once('='))
            .map(|(scheme, address)| (scheme.trim().to_ascii_lowercase(), address.trim()))
            .collect::<Vec<_>>();
        entries
            .iter()
            .find(|(scheme, _)| scheme == "https")
            .or_else(|| entries.iter().find(|(scheme, _)| scheme == "http"))
            .or_else(|| entries.iter().find(|(scheme, _)| scheme == "socks"))
            .map(|(scheme, address)| (scheme.clone(), *address))?
    } else {
        ("http".to_string(), configured)
    };
    if selected.1.is_empty() {
        return None;
    }
    if selected.1.contains("://") {
        Some(selected.1.to_string())
    } else if selected.0 == "socks" {
        Some(format!("socks5://{}", selected.1))
    } else {
        Some(format!("http://{}", selected.1))
    }
}

#[cfg(windows)]
fn parse_windows_proxy_bypass(configured: &str) -> Option<String> {
    let entries = configured
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && !entry.eq_ignore_ascii_case("<local>"))
        .collect::<Vec<_>>();
    (!entries.is_empty()).then(|| entries.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    #[test]
    fn retry_policy_only_accepts_explicit_transient_statuses() {
        for status in [429, 502, 503, 504] {
            assert!(retryable_status(StatusCode::from_u16(status).unwrap()));
        }
        for status in [400, 401, 403, 404, 500, 501, 505] {
            assert!(!retryable_status(StatusCode::from_u16(status).unwrap()));
        }
    }

    #[test]
    fn exponential_backoff_is_bounded() {
        let policy = ManagedRuntimeDownloadPolicy {
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(250),
            ..Default::default()
        };
        assert_eq!(retry_backoff(&policy, 1), Duration::from_millis(100));
        assert_eq!(retry_backoff(&policy, 2), Duration::from_millis(200));
        assert_eq!(retry_backoff(&policy, 3), Duration::from_millis(250));
        assert_eq!(retry_backoff(&policy, 20), Duration::from_millis(250));
    }

    #[cfg(windows)]
    #[test]
    fn windows_proxy_settings_support_single_and_per_protocol_values() {
        assert_eq!(
            parse_windows_proxy_server("127.0.0.1:7897").as_deref(),
            Some("http://127.0.0.1:7897")
        );
        assert_eq!(
            parse_windows_proxy_server("http=proxy:80;https=secure-proxy:443").as_deref(),
            Some("http://secure-proxy:443")
        );
        assert_eq!(
            parse_windows_proxy_server("socks=127.0.0.1:1080").as_deref(),
            Some("socks5://127.0.0.1:1080")
        );
        assert_eq!(
            parse_windows_proxy_bypass("localhost;*.internal;<local>").as_deref(),
            Some("localhost,*.internal")
        );
    }

    #[tokio::test]
    async fn transient_http_failures_retry_then_return_a_verified_artifact() {
        let body = b"verified runtime archive".to_vec();
        let responses = vec![(503, Vec::new()), (429, Vec::new()), (200, body.clone())];
        let (url, attempts, server) = spawn_sequence_server(responses).await;
        let directory = test_directory("retry-success");
        let policy = ManagedRuntimeDownloadPolicy {
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(2),
            max_attempts: 3,
            ..Default::default()
        };
        let downloader = ManagedRuntimeDownloader::new("OpenTopia test", policy).unwrap();
        let artifact = ManagedRuntimeArtifact {
            url,
            file_name: "runtime.zip".to_string(),
            expected_sha256: sha256(&body),
            download_directory: directory.clone(),
            max_bytes: 1024,
        };

        let download = downloader.download_verified(&artifact).await.unwrap();
        assert_eq!(tokio::fs::read(download.path()).await.unwrap(), body);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        let downloaded_path = download.path().to_path_buf();
        drop(download);
        assert!(!downloaded_path.exists());
        server.await.unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn integrity_failure_removes_the_temporary_file() {
        let body = b"corrupt runtime archive".to_vec();
        let (url, _, server) = spawn_sequence_server(vec![(200, body)]).await;
        let directory = test_directory("hash-cleanup");
        let policy = ManagedRuntimeDownloadPolicy {
            initial_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
            max_attempts: 1,
            ..Default::default()
        };
        let downloader = ManagedRuntimeDownloader::new("OpenTopia test", policy).unwrap();
        let artifact = ManagedRuntimeArtifact {
            url,
            file_name: "runtime.zip".to_string(),
            expected_sha256: "0".repeat(64),
            download_directory: directory.clone(),
            max_bytes: 1024,
        };

        let error = downloader
            .download_verified(&artifact)
            .await
            .expect_err("hash mismatch must fail");
        assert!(error.to_string().contains("download failed"));
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        server.await.unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    fn sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect()
    }

    fn test_directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "opentopia-managed-download-{label}-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    async fn spawn_sequence_server(
        responses: Vec<(u16, Vec<u8>)>,
    ) -> (Url, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_server = Arc::clone(&attempts);
        let server = tokio::spawn(async move {
            for (status, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                attempts_for_server.fetch_add(1, Ordering::SeqCst);
                let mut request = [0_u8; 4096];
                let _ = socket.read(&mut request).await.unwrap();
                let reason = match status {
                    200 => "OK",
                    429 => "Too Many Requests",
                    503 => "Service Unavailable",
                    _ => "Error",
                };
                let header = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(header.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        (
            Url::parse(&format!("http://{address}/runtime.zip")).unwrap(),
            attempts,
            server,
        )
    }
}
