use crate::execution_spec::{EnvironmentPolicy, ExecutionSpec};
use crate::git_workflow::GIT_NONINTERACTIVE_ENVIRONMENT;
use crate::sandbox::NetworkPolicy;
use std::path::Path;

/// Applies deterministic behavior for tools with well-known non-interactive
/// contracts. Correct containment and lifecycle behavior never depends on an
/// adapter; an unknown tool simply runs with the generic execution contract.
pub(crate) fn adapt(mut spec: ExecutionSpec, cwd: &Path) -> ExecutionSpec {
    let executable = Path::new(&spec.program)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(&spec.program)
        .to_ascii_lowercase();
    match executable.as_str() {
        "git" => adapt_git(&mut spec, cwd),
        "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe" => adapt_powershell(&mut spec),
        _ => {}
    }
    spec
}

fn adapt_git(spec: &mut ExecutionSpec, cwd: &Path) {
    spec.environment = EnvironmentPolicy::Isolated;
    spec.clear_env = true;
    spec.env
        .extend(GIT_NONINTERACTIVE_ENVIRONMENT.map(|(key, value)| (key.into(), value.into())));

    let command = git_subcommand(&spec.args).map(str::to_string);
    let safe_directory = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let null_device = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let mut prefix = vec![
        "--no-pager".to_string(),
        "-c".to_string(),
        format!("core.hooksPath={null_device}"),
        "-c".to_string(),
        "core.fsmonitor=false".to_string(),
        "-c".to_string(),
        format!("safe.directory={}", safe_directory.to_string_lossy()),
    ];
    prefix.append(&mut spec.args);
    spec.args = prefix;

    if command.as_deref().is_some_and(git_writes_metadata) {
        spec.requirements.write_paths.push(cwd.join(".git"));
    }
    if command.as_deref().is_some_and(git_uses_network) {
        spec.requirements.network = Some(NetworkPolicy::Allow);
    }
}

fn git_subcommand(args: &[String]) -> Option<&str> {
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-c" | "--config-env" | "-C" | "--git-dir" | "--work-tree" => index += 2,
            value if value.starts_with('-') => index += 1,
            value => return Some(value),
        }
    }
    None
}

fn git_writes_metadata(command: &str) -> bool {
    matches!(
        command,
        "add"
            | "branch"
            | "checkout"
            | "commit"
            | "fetch"
            | "merge"
            | "pull"
            | "rebase"
            | "reset"
            | "restore"
            | "switch"
            | "tag"
            | "worktree"
    )
}

fn git_uses_network(command: &str) -> bool {
    matches!(
        command,
        "clone" | "fetch" | "ls-remote" | "pull" | "push" | "submodule"
    )
}

fn adapt_powershell(spec: &mut ExecutionSpec) {
    let has_no_profile = spec
        .args
        .iter()
        .any(|arg| arg.eq_ignore_ascii_case("-NoProfile"));
    if !has_no_profile {
        spec.args.insert(0, "-NoProfile".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::adapt;
    use crate::execution_spec::ExecutionSpec;
    use crate::sandbox::NetworkPolicy;
    use std::path::Path;

    #[test]
    fn git_adapter_is_noninteractive_and_detects_metadata_writes() {
        let spec = adapt(
            ExecutionSpec::new("git").args(["commit", "-m", "message"]),
            Path::new(r"C:\workspace"),
        );
        assert_eq!(
            spec.env.get(std::ffi::OsStr::new("GIT_TERMINAL_PROMPT")),
            Some(&"0".into())
        );
        assert!(spec
            .requirements
            .write_paths
            .iter()
            .any(|path| path.ends_with(".git")));
        assert!(spec.args.iter().any(|arg| arg == "core.fsmonitor=false"));
    }

    #[test]
    fn git_adapter_detects_network_commands() {
        let spec = adapt(
            ExecutionSpec::new("git").args(["fetch", "origin"]),
            Path::new(r"C:\workspace"),
        );
        assert_eq!(spec.requirements.network, Some(NetworkPolicy::Allow));
    }
}
