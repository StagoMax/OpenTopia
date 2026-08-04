//! First-party Windows command sandbox used by OpenTopia.
//!
//! The executable deliberately accepts only a structured launch request with
//! absolute policy paths. Its elevated backend uses dedicated offline/online users; its
//! fallback uses a WRITE_RESTRICTED token and intentionally advertises only
//! write containment. Both contain the target process tree in a kill-on-close
//! Job Object.

use anyhow::Context;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;

/// Reserved by the helper for failures in the sandbox infrastructure itself.
/// Target process exit codes remain ordinary command results unless stderr also
/// contains the versioned marker below.
pub const SANDBOX_ERROR_EXIT_CODE: i32 = 125;
pub const SANDBOX_ERROR_PREFIX: &str = "OPENTOPIA_SANDBOX_ERROR ";
pub const SANDBOX_ERROR_NONCE_ENV: &str = "OPENTOPIA_SANDBOX_ERROR_NONCE";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum NetworkMode {
    Deny,
    Internet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum BackendMode {
    Auto,
    Elevated,
    Unelevated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SandboxRequest {
    interactive: bool,
    cwd: PathBuf,
    read_roots: Vec<PathBuf>,
    runtime_roots: Vec<PathBuf>,
    write_roots: Vec<PathBuf>,
    protected_paths: Vec<PathBuf>,
    denied_read_paths: Vec<PathBuf>,
    allowed_protected_roots: Vec<PathBuf>,
    network: NetworkMode,
    timeout_ms: Option<u64>,
    termination_timeout_ms: u64,
    max_memory_bytes: Option<u64>,
    max_cpu_time_ms: Option<u64>,
    max_output_bytes: Option<u64>,
    backend: BackendMode,
    command: Vec<String>,
}

pub fn run_from_env() -> Result<i32> {
    let all_args = env::args().skip(1).collect::<Vec<_>>();
    if all_args.first().map(String::as_str) == Some("protocol") {
        let extra = all_args.get(1).map(String::as_str);
        if extra.is_some() && extra != Some("--json") {
            anyhow::bail!("usage: opentopia-sandbox protocol [--json]");
        }
        println!(
            "{}",
            serde_json::to_string(&opentopia_sandbox_protocol::SandboxProtocolInfo::current(
                env!("CARGO_PKG_VERSION")
            ))?
        );
        return Ok(0);
    }
    #[cfg(windows)]
    if all_args.first().map(String::as_str) == Some("setup") {
        return setup::run_setup(&all_args[1..]);
    }
    #[cfg(windows)]
    if all_args.first().map(String::as_str) == Some("runner") {
        return windows::run_elevated_runner(&all_args[1..]);
    }
    #[cfg(windows)]
    if all_args.first().map(String::as_str) == Some("cleanup") {
        return windows::cleanup_workspace_acl(&all_args[1..]);
    }
    let request = parse_request(all_args)?;
    #[cfg(windows)]
    {
        windows::run(request)
    }

    #[cfg(not(windows))]
    {
        let _ = request;
        anyhow::bail!("the OpenTopia Windows sandbox can run only on Windows")
    }
}

pub fn log_failure(message: &str) {
    #[cfg(windows)]
    logging::event("failure", message);
    #[cfg(not(windows))]
    let _ = message;
}

fn parse_request(args: impl IntoIterator<Item = String>) -> Result<SandboxRequest> {
    let mut args = args.into_iter().collect::<Vec<_>>().into_iter();
    match args.next().as_deref() {
        Some("run") => {}
        Some("--version") => {
            println!("opentopia-sandbox {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        _ => anyhow::bail!(
            "usage: opentopia-sandbox run --cwd <absolute-path> [--interactive] [--read-root <absolute-path>] [--runtime-root <absolute-path>] [--write-root <absolute-path>] [--protect <absolute-path>] [--timeout-ms <milliseconds>] [--termination-timeout-ms <milliseconds>] --network <deny|internet> -- <program> [args...]"
        ),
    }

    let mut interactive = false;
    let mut cwd = None;
    let mut read_roots = Vec::new();
    let mut runtime_roots = Vec::new();
    let mut write_roots = Vec::new();
    let mut protected_paths = Vec::new();
    let mut denied_read_paths = Vec::new();
    let mut allowed_protected_roots = Vec::new();
    let mut network = None;
    let mut timeout_ms = None;
    let mut termination_timeout_ms = 5_000;
    let mut max_memory_bytes = None;
    let mut max_cpu_time_ms = None;
    let mut max_output_bytes = None;
    let mut backend = BackendMode::Auto;
    let mut command = Vec::new();

    while let Some(arg) = args.next() {
        if arg == "--" {
            command.extend(args);
            break;
        }
        match arg.as_str() {
            "--interactive" => interactive = true,
            "--cwd" => cwd = Some(absolute_path(next_value("--cwd", &mut args)?)?),
            "--read-root" => read_roots.push(absolute_path(next_value("--read-root", &mut args)?)?),
            "--runtime-root" => {
                runtime_roots.push(absolute_path(next_value("--runtime-root", &mut args)?)?)
            }
            "--write-root" => {
                write_roots.push(absolute_path(next_value("--write-root", &mut args)?)?)
            }
            "--protect" => {
                protected_paths.push(absolute_path(next_value("--protect", &mut args)?)?)
            }
            "--deny-read" => {
                denied_read_paths.push(absolute_path(next_value("--deny-read", &mut args)?)?)
            }
            "--allow-protected-root" => allowed_protected_roots.push(absolute_path(next_value(
                "--allow-protected-root",
                &mut args,
            )?)?),
            "--network" => {
                network = Some(match next_value("--network", &mut args)?.as_str() {
                    "deny" => NetworkMode::Deny,
                    "internet" => NetworkMode::Internet,
                    value => anyhow::bail!("unsupported network mode: {value}"),
                });
            }
            "--timeout-ms" => {
                timeout_ms = Some(parse_timeout("--timeout-ms", &mut args)?);
            }
            "--termination-timeout-ms" => {
                termination_timeout_ms = parse_timeout("--termination-timeout-ms", &mut args)?;
            }
            "--max-memory-bytes" => {
                max_memory_bytes = Some(parse_timeout("--max-memory-bytes", &mut args)?);
            }
            "--max-cpu-time-ms" => {
                max_cpu_time_ms = Some(parse_timeout("--max-cpu-time-ms", &mut args)?);
            }
            "--max-output-bytes" => {
                max_output_bytes = Some(parse_quantity("--max-output-bytes", &mut args)?);
            }
            "--backend" => {
                backend = match next_value("--backend", &mut args)?.as_str() {
                    "auto" => BackendMode::Auto,
                    "elevated" => BackendMode::Elevated,
                    "unelevated" | "legacy" => BackendMode::Unelevated,
                    value => anyhow::bail!("unsupported Windows sandbox backend: {value}"),
                };
            }
            _ => anyhow::bail!("unexpected sandbox argument: {arg}"),
        }
    }

    let cwd = cwd.context("missing required --cwd")?;
    if command.is_empty() {
        anyhow::bail!("missing sandboxed program after --")
    }
    Ok(SandboxRequest {
        interactive,
        cwd,
        read_roots,
        runtime_roots,
        write_roots,
        protected_paths,
        denied_read_paths,
        allowed_protected_roots,
        network: network.context("missing required --network")?,
        timeout_ms,
        termination_timeout_ms,
        max_memory_bytes,
        max_cpu_time_ms,
        max_output_bytes,
        backend,
        command,
    })
}

fn parse_timeout(flag: &str, args: &mut std::vec::IntoIter<String>) -> Result<u64> {
    let value = next_value(flag, args)?;
    let timeout = value
        .parse::<u64>()
        .with_context(|| format!("invalid millisecond value for {flag}: {value}"))?;
    if timeout == 0 {
        anyhow::bail!("{flag} must be greater than zero")
    }
    Ok(timeout)
}

fn parse_quantity(flag: &str, args: &mut std::vec::IntoIter<String>) -> Result<u64> {
    let value = next_value(flag, args)?;
    value
        .parse::<u64>()
        .with_context(|| format!("invalid numeric value for {flag}: {value}"))
}

fn next_value(flag: &str, args: &mut std::vec::IntoIter<String>) -> Result<String> {
    args.next()
        .with_context(|| format!("missing value for {flag}"))
}

fn absolute_path(value: String) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        anyhow::bail!("sandbox paths must be absolute: {}", path.display())
    }
    path.canonicalize()
        .with_context(|| format!("sandbox path must exist: {}", path.display()))
}

#[cfg(windows)]
#[path = "env.rs"]
mod process_env;

#[cfg(windows)]
mod dpapi;

#[cfg(windows)]
mod setup;

#[cfg(windows)]
mod logging;

#[cfg(windows)]
mod wfp;

#[cfg(windows)]
mod windows;

#[cfg(test)]
mod tests {
    use super::parse_request;
    use super::NetworkMode;

    #[test]
    fn parses_a_structured_run_request() {
        let cwd = std::env::current_dir().expect("current directory");
        let cwd = cwd.to_string_lossy().to_string();
        let request = parse_request(
            vec![
                "run",
                "--cwd",
                &cwd,
                "--interactive",
                "--read-root",
                &cwd,
                "--write-root",
                &cwd,
                "--runtime-root",
                &cwd,
                "--protect",
                &cwd,
                "--deny-read",
                &cwd,
                "--network",
                "deny",
                "--backend",
                "unelevated",
                "--timeout-ms",
                "2500",
                "--termination-timeout-ms",
                "7000",
                "--max-memory-bytes",
                "1048576",
                "--max-cpu-time-ms",
                "9000",
                "--max-output-bytes",
                "65536",
                "--",
                "cmd.exe",
                "/d",
                "/c",
                "echo ok",
            ]
            .into_iter()
            .map(str::to_owned),
        )
        .expect("parse request");

        assert_eq!(request.network, NetworkMode::Deny);
        assert!(request.interactive);
        assert_eq!(request.timeout_ms, Some(2_500));
        assert_eq!(request.runtime_roots.len(), 1);
        assert_eq!(request.denied_read_paths.len(), 1);
        assert_eq!(request.termination_timeout_ms, 7_000);
        assert_eq!(request.max_memory_bytes, Some(1_048_576));
        assert_eq!(request.max_cpu_time_ms, Some(9_000));
        assert_eq!(request.max_output_bytes, Some(65_536));
        assert_eq!(request.backend, super::BackendMode::Unelevated);
        assert_eq!(request.command, ["cmd.exe", "/d", "/c", "echo ok"]);
    }
}
