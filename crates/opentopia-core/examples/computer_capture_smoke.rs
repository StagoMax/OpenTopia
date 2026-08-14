//! Manual Windows smoke test for background-window capture.
//!
//! Run with an interactive desktop and a non-foreground File Explorer window:
//! `cargo run -p opentopia-core --example computer_capture_smoke`

use opentopia_core::{ComputerRuntime, ComputerSessionId, LocalComputerRuntime, ObserveOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let runtime = LocalComputerRuntime::default();
    let session = ComputerSessionId::new();
    let windows = runtime.list_windows(session).await?;
    let target = windows
        .iter()
        .find(|window| {
            !window.is_foreground
                && window.executable.as_deref().is_some_and(|executable| {
                    executable
                        .rsplit(['\\', '/'])
                        .next()
                        .is_some_and(|name| name.eq_ignore_ascii_case("explorer.exe"))
                })
        })
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no background File Explorer window is available"))?;
    let observation = runtime
        .observe(session, target, ObserveOptions::default())
        .await?;
    let screenshot = observation
        .screenshot
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("observation returned no screenshot"))?;
    let signature_valid = screenshot
        .bytes
        .starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    if !signature_valid {
        anyhow::bail!("observation did not return a PNG payload");
    }

    println!(
        "windows={} background_capture={}x{} mime={} bytes={} png_signature=true",
        windows.len(),
        observation.image_width,
        observation.image_height,
        screenshot.mime_type,
        screenshot.bytes.len()
    );
    runtime.close_session(session).await?;
    Ok(())
}
