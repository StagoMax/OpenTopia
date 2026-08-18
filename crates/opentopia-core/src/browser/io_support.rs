//! Network grant enforcement, screenshot validation, and download completion helpers.

use super::cdp_transport::CdpPage;
use super::{BrowserDownload, BrowserError};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::RwLock as StdRwLock;
use std::time::Duration;

pub(super) fn normalize_network_host(raw_host: &str) -> Result<String, BrowserError> {
    let host = raw_host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty()
        || host.contains('/')
        || host.contains('@')
        || host.contains(char::is_whitespace)
    {
        return Err(BrowserError::InvalidUrl(format!(
            "invalid network host `{raw_host}`"
        )));
    }
    let parsed = reqwest::Url::parse(&format!("http://[{host}]/"))
        .or_else(|_| reqwest::Url::parse(&format!("http://{host}/")))
        .map_err(|_| BrowserError::InvalidUrl(format!("invalid network host `{raw_host}`")))?;
    if parsed.port().is_some() {
        return Err(BrowserError::InvalidUrl(format!(
            "network host must not include a port: `{raw_host}`"
        )));
    }
    parsed
        .host_str()
        .map(|host| host.trim_matches(['[', ']']).to_ascii_lowercase())
        .ok_or_else(|| BrowserError::InvalidUrl(format!("invalid network host `{raw_host}`")))
}

pub(super) fn network_request_host(raw_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(raw_url).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.host_str()
        .and_then(|host| normalize_network_host(host).ok())
}

pub(super) fn network_policy_allows_url(
    policy: &StdRwLock<Option<HashSet<String>>>,
    raw_url: &str,
) -> bool {
    let Ok(policy) = policy.read() else {
        return false;
    };
    let Some(allowed_hosts) = policy.as_ref() else {
        return true;
    };
    network_request_host(raw_url).is_some_and(|host| allowed_hosts.contains(&host))
}

pub(super) fn png_looks_blank(bytes: &[u8]) -> Result<bool, BrowserError> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| BrowserError::Protocol(format!("Invalid screenshot PNG: {error}")))?;
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|error| BrowserError::Protocol(format!("Invalid screenshot PNG: {error}")))?;
    if info.bit_depth != png::BitDepth::Eight {
        return Ok(false);
    }
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Ok(false),
    };
    let pixels = &pixels[..info.buffer_size()];
    let pixel_count = pixels.len() / channels;
    if pixel_count == 0 {
        return Ok(true);
    }
    let stride = (pixel_count / 4096).max(1);
    let mut sampled = 0_usize;
    let mut blank = 0_usize;
    for pixel_index in (0..pixel_count).step_by(stride) {
        let pixel = &pixels[pixel_index * channels..][..channels];
        let (red, green, blue, alpha) = match info.color_type {
            png::ColorType::Grayscale => (pixel[0], pixel[0], pixel[0], 255),
            png::ColorType::GrayscaleAlpha => (pixel[0], pixel[0], pixel[0], pixel[1]),
            png::ColorType::Rgb => (pixel[0], pixel[1], pixel[2], 255),
            png::ColorType::Rgba => (pixel[0], pixel[1], pixel[2], pixel[3]),
            png::ColorType::Indexed => unreachable!(),
        };
        sampled += 1;
        if alpha <= 2 || (red <= 3 && green <= 3 && blue <= 3) {
            blank += 1;
        }
    }
    Ok(blank as f64 / sampled as f64 >= 0.995)
}

pub(super) async fn list_downloads(directory: &Path) -> Result<HashSet<PathBuf>, BrowserError> {
    let mut entries = tokio::fs::read_dir(directory).await?;
    let mut paths = HashSet::new();
    while let Some(entry) = entries.next_entry().await? {
        paths.insert(entry.path());
    }
    Ok(paths)
}

pub(super) async fn wait_for_download(
    page: &mut CdpPage,
    directory: &Path,
    before: &HashSet<PathBuf>,
    expected_filename: Option<&str>,
    timeout: Duration,
    maximum_bytes: u64,
) -> Result<BrowserDownload, BrowserError> {
    let started = tokio::time::Instant::now();
    let mut last_candidate: Option<(PathBuf, u64)> = None;
    let mut download_guid = None;
    let mut protocol_completed = false;
    loop {
        while let Some(event) = page.try_next_event()? {
            match event.method.as_str() {
                "Browser.downloadWillBegin" => {
                    download_guid = event
                        .params
                        .get("guid")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                "Browser.downloadProgress" => {
                    let event_guid = event.params.get("guid").and_then(Value::as_str);
                    if download_guid
                        .as_deref()
                        .is_none_or(|guid| event_guid == Some(guid))
                    {
                        let received = event
                            .params
                            .get("receivedBytes")
                            .and_then(Value::as_f64)
                            .unwrap_or_default()
                            .max(0.0) as u64;
                        if received > maximum_bytes {
                            cancel_browser_download(page, download_guid.as_deref()).await;
                            return Err(BrowserError::DownloadTooLarge {
                                maximum: maximum_bytes,
                            });
                        }
                        match event.params.get("state").and_then(Value::as_str) {
                            Some("completed") => protocol_completed = true,
                            Some("canceled") => {
                                return Err(BrowserError::Protocol(
                                    "Browser download was canceled".to_string(),
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        let mut entries = tokio::fs::read_dir(directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if before.contains(&path) {
                continue;
            }
            let metadata = entry.metadata().await?;
            if !metadata.is_file() {
                continue;
            }
            let bytes = metadata.len();
            if bytes > maximum_bytes {
                cancel_browser_download(page, download_guid.as_deref()).await;
                return Err(BrowserError::DownloadTooLarge {
                    maximum: maximum_bytes,
                });
            }
            if matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("crdownload" | "tmp")
            ) {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().to_string();
            if expected_filename.is_some_and(|expected| expected != filename) {
                continue;
            }
            if protocol_completed || last_candidate.as_ref() == Some(&(path.clone(), bytes)) {
                return Ok(BrowserDownload {
                    content_type: content_type_for_path(&path),
                    path,
                    filename,
                    bytes,
                });
            }
            last_candidate = Some((path, bytes));
        }
        if started.elapsed() >= timeout {
            cancel_browser_download(page, download_guid.as_deref()).await;
            return Err(BrowserError::DownloadTimeout);
        }
        tokio::select! {
            event = page.next_event() => {
                if let Some(event) = event? {
                    page.push_event(event);
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(150)) => {}
        }
    }
}

pub(super) async fn cancel_browser_download(page: &CdpPage, guid: Option<&str>) {
    if let Some(guid) = guid {
        let _ = page
            .root_command("Browser.cancelDownload", json!({ "guid": guid }))
            .await;
    }
}

pub(super) fn content_type_for_path(path: &Path) -> Option<String> {
    match path.extension().and_then(|extension| extension.to_str())? {
        "csv" => Some("text/csv".to_string()),
        "json" => Some("application/json".to_string()),
        "pdf" => Some("application/pdf".to_string()),
        "png" => Some("image/png".to_string()),
        "jpg" | "jpeg" => Some("image/jpeg".to_string()),
        "txt" | "log" => Some("text/plain".to_string()),
        "zip" => Some("application/zip".to_string()),
        _ => None,
    }
}
