//! First-party Windows command sandbox used by OpenTopia.
//!
//! The executable deliberately accepts only a structured, absolute-path launch
//! request. It creates an AppContainer process identity, grants the
//! request's roots to that one-use identity, and contains the target process
//! tree in a kill-on-close job object.

use anyhow::Context;
use anyhow::Result;
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkMode {
    Deny,
    Internet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SandboxRequest {
    cwd: PathBuf,
    read_roots: Vec<PathBuf>,
    write_roots: Vec<PathBuf>,
    protected_paths: Vec<PathBuf>,
    network: NetworkMode,
    command: Vec<String>,
}

pub fn run_from_env() -> Result<i32> {
    let request = parse_request(env::args().skip(1))?;
    #[cfg(windows)]
    {
        return windows::run(request);
    }

    #[cfg(not(windows))]
    {
        let _ = request;
        anyhow::bail!("the OpenTopia Windows sandbox can run only on Windows")
    }
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
            "usage: opentopia-sandbox run --cwd <absolute-path> [--read-root <absolute-path>] [--write-root <absolute-path>] [--protect <absolute-path>] --network <deny|internet> -- <program> [args...]"
        ),
    }

    let mut cwd = None;
    let mut read_roots = Vec::new();
    let mut write_roots = Vec::new();
    let mut protected_paths = Vec::new();
    let mut network = None;
    let mut command = Vec::new();

    while let Some(arg) = args.next() {
        if arg == "--" {
            command.extend(args);
            break;
        }
        match arg.as_str() {
            "--cwd" => cwd = Some(absolute_path(next_value("--cwd", &mut args)?)?),
            "--read-root" => read_roots.push(absolute_path(next_value("--read-root", &mut args)?)?),
            "--write-root" => {
                write_roots.push(absolute_path(next_value("--write-root", &mut args)?)?)
            }
            "--protect" => {
                protected_paths.push(absolute_path(next_value("--protect", &mut args)?)?)
            }
            "--network" => {
                network = Some(match next_value("--network", &mut args)?.as_str() {
                    "deny" => NetworkMode::Deny,
                    "internet" => NetworkMode::Internet,
                    value => anyhow::bail!("unsupported network mode: {value}"),
                });
            }
            _ => anyhow::bail!("unexpected sandbox argument: {arg}"),
        }
    }

    let cwd = cwd.context("missing required --cwd")?;
    if command.is_empty() {
        anyhow::bail!("missing sandboxed program after --")
    }
    Ok(SandboxRequest {
        cwd,
        read_roots,
        write_roots,
        protected_paths,
        network: network.context("missing required --network")?,
        command,
    })
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
                "--read-root",
                &cwd,
                "--write-root",
                &cwd,
                "--protect",
                &cwd,
                "--network",
                "deny",
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
        assert_eq!(request.command, ["cmd.exe", "/d", "/c", "echo ok"]);
    }
}
