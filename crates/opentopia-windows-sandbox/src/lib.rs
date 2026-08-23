//! First-party Windows command sandbox used by OpenTopia.
//!
//! The executable deliberately accepts only a structured launch request with
//! absolute policy paths. Its dedicated-user backend uses isolated offline/online users; its
//! fallback uses a WRITE_RESTRICTED token and intentionally advertises only
//! write containment. Both contain the target process tree in a kill-on-close
//! Job Object.

use anyhow::Context;
use anyhow::Result;
use opentopia_sandbox_protocol::{FilesystemCapabilities, ReadExecuteCapability, ReadProvisioning};
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
    DedicatedUser,
    Unelevated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SandboxRequest {
    interactive: bool,
    cwd: PathBuf,
    filesystem: FilesystemCapabilities,
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
        return windows::run_dedicated_user_runner(&all_args[1..]);
    }
    #[cfg(windows)]
    if all_args.first().map(String::as_str) == Some("registry-provision") {
        return windows::provision_runtime_registry(&all_args[1..]);
    }
    #[cfg(windows)]
    if all_args.first().map(String::as_str) == Some("canary") {
        return windows::run_setup_canary(&all_args[1..]);
    }
    #[cfg(windows)]
    if all_args.first().map(String::as_str) == Some("teardown") {
        return setup::run_teardown(&all_args[1..]);
    }
    #[cfg(windows)]
    if all_args.first().map(String::as_str) == Some("cleanup") {
        return windows::cleanup_workspace_acl(&all_args[1..]);
    }
    #[cfg(windows)]
    if all_args.first().map(String::as_str) == Some("provision") {
        let mut provision_args = all_args;
        provision_args[0] = "run".to_string();
        provision_args.extend([
            "--".to_string(),
            "cmd.exe".to_string(),
            "/d".to_string(),
            "/c".to_string(),
            "exit 0".to_string(),
        ]);
        return windows::provision(parse_request(provision_args)?);
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
            "usage: opentopia-sandbox run --cwd <absolute-path> [--interactive] [--read-root <absolute-path>] [--managed-runtime-root <absolute-path>] [--runtime-root <absolute-path>] [--write-root <absolute-path>] [--runtime-home <absolute-path>] [--protect <absolute-path>] [--timeout-ms <milliseconds>] [--termination-timeout-ms <milliseconds>] --network <deny|internet> -- <program> [args...]"
        ),
    }

    let mut interactive = false;
    let mut cwd = None;
    let mut read_roots = Vec::new();
    let mut managed_runtime_roots = Vec::new();
    let mut runtime_roots = Vec::new();
    let mut write_roots = Vec::new();
    let mut runtime_home = None;
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
            "--managed-runtime-root" => managed_runtime_roots.push(absolute_path(next_value(
                "--managed-runtime-root",
                &mut args,
            )?)?),
            "--runtime-root" => {
                runtime_roots.push(absolute_path(next_value("--runtime-root", &mut args)?)?)
            }
            "--write-root" => {
                write_roots.push(absolute_path(next_value("--write-root", &mut args)?)?)
            }
            "--runtime-home" => {
                runtime_home = Some(absolute_path(next_value("--runtime-home", &mut args)?)?)
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
                    "dedicated-user" | "dedicated_user" | "elevated" => BackendMode::DedicatedUser,
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
    if let Some(home) = runtime_home.as_ref() {
        if !write_roots.iter().any(|root| root == home) {
            write_roots.push(home.clone());
        }
    }
    let mut read_execute = Vec::<ReadExecuteCapability>::new();
    for (path, provisioning) in read_roots
        .into_iter()
        .map(|path| (path, ReadProvisioning::Managed))
        .chain(
            runtime_roots
                .into_iter()
                .map(|path| (path, ReadProvisioning::ExistingOnly)),
        )
    {
        if let Some(existing) = read_execute
            .iter_mut()
            .find(|capability| capability.path == path)
        {
            if provisioning == ReadProvisioning::ExistingOnly {
                existing.provisioning = ReadProvisioning::ExistingOnly;
            }
        } else {
            read_execute.push(ReadExecuteCapability { path, provisioning });
        }
    }
    for managed in read_execute
        .iter()
        .filter(|capability| capability.provisioning == ReadProvisioning::Managed)
    {
        if let Some(external) = read_execute.iter().find(|capability| {
            capability.provisioning == ReadProvisioning::ExistingOnly
                && path_is_within(&capability.path, &managed.path)
        }) {
            anyhow::bail!(
                "managed read root {} contains immutable external runtime {}; classify the runtime as managed or narrow the managed root",
                managed.path.display(),
                external.path.display()
            )
        }
    }
    managed_runtime_roots.sort();
    managed_runtime_roots.dedup();
    for root in &managed_runtime_roots {
        anyhow::ensure!(
            root.is_dir(),
            "managed runtime root does not exist or is not a directory: {}",
            root.display()
        );
        anyhow::ensure!(
            read_execute.iter().any(|capability| {
                capability.provisioning == ReadProvisioning::Managed
                    && path_is_within(&capability.path, root)
            }),
            "managed runtime root {} does not contain any managed read capability",
            root.display()
        );
    }
    Ok(SandboxRequest {
        interactive,
        cwd,
        filesystem: FilesystemCapabilities {
            read_execute,
            managed_runtime_roots,
            write: write_roots,
            deny_read: denied_read_paths,
            deny_write: protected_paths,
            allow_protected_write: allowed_protected_roots,
            runtime_home,
        },
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

fn path_is_within(path: &std::path::Path, root: &std::path::Path) -> bool {
    path == root || path.starts_with(root)
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
                "--runtime-home",
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
        assert_eq!(
            request
                .filesystem
                .read_execute
                .iter()
                .filter(|capability| {
                    capability.provisioning
                        == opentopia_sandbox_protocol::ReadProvisioning::ExistingOnly
                })
                .count(),
            1
        );
        assert_eq!(request.filesystem.read_execute.len(), 1);
        assert_eq!(request.filesystem.deny_read.len(), 1);
        assert_eq!(
            request.filesystem.runtime_home.as_deref(),
            Some(request.cwd.as_path())
        );
        assert_eq!(request.termination_timeout_ms, 7_000);
        assert_eq!(request.max_memory_bytes, Some(1_048_576));
        assert_eq!(request.max_cpu_time_ms, Some(9_000));
        assert_eq!(request.max_output_bytes, Some(65_536));
        assert_eq!(request.backend, super::BackendMode::Unelevated);
        assert_eq!(request.command, ["cmd.exe", "/d", "/c", "echo ok"]);
    }

    #[test]
    fn managed_runtime_root_must_own_a_managed_read_capability() {
        let root = std::env::temp_dir().join(format!(
            "opentopia-managed-runtime-{}",
            uuid::Uuid::new_v4()
        ));
        let generation = root.join("10.30.0");
        std::fs::create_dir_all(&generation).expect("create managed runtime fixture");
        let root_arg = root.to_string_lossy().into_owned();
        let generation_arg = generation.to_string_lossy().into_owned();
        let request = parse_request(
            vec![
                "run".to_string(),
                "--cwd".to_string(),
                root_arg.clone(),
                "--read-root".to_string(),
                generation_arg,
                "--managed-runtime-root".to_string(),
                root_arg,
                "--network".to_string(),
                "internet".to_string(),
                "--".to_string(),
                "cmd.exe".to_string(),
            ]
            .into_iter(),
        )
        .expect("parse managed runtime request");

        assert_eq!(request.filesystem.managed_runtime_roots.len(), 1);
        assert_eq!(request.filesystem.read_execute.len(), 1);
        std::fs::remove_dir_all(root).expect("remove managed runtime fixture");
    }
}
