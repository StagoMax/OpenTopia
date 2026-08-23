use crate::execution_spec::{
    EnvironmentPolicy, ExecutionCapability, ExecutionFailure, ExecutionSpec, ExecutionStage,
};
use crate::sandbox::LocalSandboxConfig;
use crate::workspace_execution_capsule::WorkspaceExecutionCapsule;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

const SENSITIVE_CHILD_ENV_KEYS: &[&str] = &[
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENTOPIA_API_KEY",
    "OPENTOPIA_API_TOKEN",
    "CREDIT_REVIEW_LLM_API_KEY",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AZURE_CLIENT_SECRET",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "SSH_AUTH_SOCK",
    "CARGO_HOME",
    "GRADLE_USER_HOME",
    "NPM_CONFIG_USERCONFIG",
    "DOCKER_CONFIG",
    "KUBECONFIG",
    "AWS_SHARED_CREDENTIALS_FILE",
    "AZURE_CONFIG_DIR",
    "GIT_CONFIG_GLOBAL",
    "GNUPGHOME",
];

const WINDOWS_BASE_ENV: &[&str] = &[
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "PATH",
    "PATHEXT",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "HOMEDRIVE",
    "HOMEPATH",
];

const UNIX_BASE_ENV: &[&str] = &["PATH", "LANG", "LC_ALL", "LC_CTYPE", "SHELL"];

#[derive(Debug)]
pub(crate) struct ResolvedRuntime {
    pub program: PathBuf,
    pub read_roots: Vec<PathBuf>,
    pub managed_runtime_roots: Vec<PathBuf>,
    pub sandbox_home: Option<PathBuf>,
    pub environment: Vec<(OsString, OsString)>,
}

pub(crate) fn resolve_runtime(
    request: &ExecutionSpec,
    cwd: &Path,
    workspace_root: &Path,
    config: &LocalSandboxConfig,
    capsule: &WorkspaceExecutionCapsule,
) -> Result<ResolvedRuntime, ExecutionFailure> {
    let capsule = request
        .requires_capability(ExecutionCapability::WorkspaceShell)
        .then_some(capsule);
    let environment = resolve_execution_environment(request, capsule)?;
    let program = resolve_executable(&request.program, cwd, &environment)?;
    let mut roots = BTreeSet::new();
    let mut managed_runtime_roots = BTreeSet::new();
    if let Some(parent) = program.parent() {
        roots.insert(canonical_or_original(parent));
    }
    for root in &request.runtime.read_roots {
        if !root.exists() {
            return Err(ExecutionFailure::without_os_error(
                ExecutionStage::ResolveRuntime,
                format!(
                    "runtime '{}' requires a missing read root: {}",
                    request.runtime.name.as_deref().unwrap_or("unnamed"),
                    root.display()
                ),
            ));
        }
        roots.insert(canonical_or_original(root));
    }
    if let Some(capsule) = capsule {
        for root in capsule.read_roots() {
            if !root.is_dir() {
                return Err(ExecutionFailure::without_os_error(
                    ExecutionStage::ResolveRuntime,
                    format!(
                        "workspace execution capsule {} requires a missing read root: {}",
                        capsule.fingerprint(),
                        root.display()
                    ),
                ));
            }
            roots.insert(canonical_or_original(root));
        }
        for root in capsule.managed_runtime_roots() {
            if root.is_dir() {
                managed_runtime_roots.insert(canonical_or_original(root));
            }
        }
    }
    roots.extend(runtime_roots_from_environment(&environment)?);

    let sandbox_home = config
        .effective_sandbox_home(workspace_root)
        .map(|home| normalized_canonical_path(&home));
    if config.is_enabled() {
        if let Some(home) = sandbox_home.as_ref() {
            prepare_sandbox_home(home)?;
        }
    }

    Ok(ResolvedRuntime {
        program,
        read_roots: roots.into_iter().collect(),
        managed_runtime_roots: managed_runtime_roots.into_iter().collect(),
        sandbox_home,
        environment,
    })
}

fn resolve_executable(
    program: &str,
    cwd: &Path,
    environment: &[(OsString, OsString)],
) -> Result<PathBuf, ExecutionFailure> {
    let requested = Path::new(program);
    if requested.is_absolute() || requested.components().count() > 1 {
        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            cwd.join(requested)
        };
        if candidate.is_file() {
            return Ok(normalized_canonical_path(&candidate));
        }
        return Err(ExecutionFailure::without_os_error(
            ExecutionStage::ResolveRuntime,
            format!("executable does not exist: {}", candidate.display()),
        ));
    }

    let path_value = environment
        .iter()
        .find(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| value.clone())
        .unwrap_or_default();

    #[cfg(windows)]
    let extensions = windows_executable_extensions(environment);
    #[cfg(not(windows))]
    let extensions = vec![OsString::new()];

    for directory in std::env::split_paths(&path_value) {
        for extension in &extensions {
            let mut name = OsString::from(program);
            if !extension.is_empty() && Path::new(program).extension().is_none() {
                name.push(extension);
            }
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Ok(normalized_canonical_path(&candidate));
            }
        }
    }

    Err(ExecutionFailure::without_os_error(
        ExecutionStage::ResolveRuntime,
        format!("executable was not found on PATH: {program}"),
    ))
}

#[cfg(windows)]
fn windows_executable_extensions(environment: &[(OsString, OsString)]) -> Vec<OsString> {
    let value = environment
        .iter()
        .find(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case("PATHEXT"))
        .map(|(_, value)| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());
    let mut extensions = vec![OsString::new()];
    extensions.extend(
        value
            .split(';')
            .filter(|value| !value.is_empty())
            .map(OsString::from),
    );
    extensions
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Canonicalizes a host path without leaking Windows' verbatim-path spelling
/// into runtime roots, executable paths, or child-process environment values.
fn normalized_canonical_path(path: &Path) -> PathBuf {
    let path = canonical_or_original(path);
    #[cfg(windows)]
    {
        let display = path.as_os_str().to_string_lossy();
        if let Some(unc) = display.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        if let Some(native) = display.strip_prefix(r"\\?\") {
            return PathBuf::from(native);
        }
    }
    path
}

fn prepare_sandbox_home(home: &Path) -> Result<(), ExecutionFailure> {
    for path in [
        home.to_path_buf(),
        home.join("AppData").join("Roaming"),
        home.join("AppData").join("Local"),
        home.join(".config"),
        home.join(".config").join("npm"),
        home.join(".config").join("pnpm"),
        home.join(".cache"),
        home.join(".cache").join("npm"),
        home.join(".local").join("share").join("pnpm"),
        home.join(".local").join("state").join("pnpm"),
        home.join("tmp"),
    ] {
        std::fs::create_dir_all(&path).map_err(|error| {
            ExecutionFailure::from_io(
                ExecutionStage::PrepareSandbox,
                format!(
                    "failed to create sandbox runtime directory {}: {error}",
                    path.display()
                ),
                &error,
            )
        })?;
    }
    Ok(())
}

fn is_runtime_root_environment_key(key: &str) -> bool {
    matches!(
        key,
        "JAVA_HOME"
            | "RUSTUP_HOME"
            | "NVM_HOME"
            | "M2_HOME"
            | "ANT_HOME"
            | "DOTNET_ROOT"
            | "PYENV_ROOT"
            | "GOROOT"
            | "VIRTUAL_ENV"
            | "CONDA_PREFIX"
    )
}

fn resolve_execution_environment(
    request: &ExecutionSpec,
    capsule: Option<&WorkspaceExecutionCapsule>,
) -> Result<Vec<(OsString, OsString)>, ExecutionFailure> {
    let mut environment = BTreeMap::<String, (OsString, OsString, bool)>::new();
    for (key, value) in inherited_environment(request) {
        let normalized = key.to_string_lossy().to_ascii_uppercase();
        if !is_sensitive_environment_key(&normalized) {
            environment.insert(normalized, (key, value, false));
        }
    }
    for (key, value) in &request.env {
        environment.insert(
            key.to_string_lossy().to_ascii_uppercase(),
            (key.clone(), value.clone(), true),
        );
    }

    if let Some(capsule) = capsule {
        for (key, value) in capsule.environment() {
            environment.insert(
                key.to_string_lossy().to_ascii_uppercase(),
                (key.clone(), value.clone(), true),
            );
        }
        prepend_path_entries(&mut environment, capsule.path_entries())?;
    }

    if let Some((key, value, _)) = environment.get("PATH").cloned() {
        // PATH is an ordered lookup contract. Filtering unusable entries must not
        // change which executable wins when multiple directories contain the same
        // command, and duplicate entries are harmless compared with silently
        // changing that precedence.
        let mut resolved = Vec::new();
        for entry in std::env::split_paths(&value) {
            if let Ok(entry) = entry.canonicalize() {
                if entry.is_dir() {
                    resolved.push(normalized_canonical_path(&entry));
                }
            }
        }
        let value = std::env::join_paths(resolved.iter()).map_err(|error| {
            ExecutionFailure::without_os_error(
                ExecutionStage::ResolveRuntime,
                format!("failed to construct the resolved runtime PATH: {error}"),
            )
        })?;
        environment.insert("PATH".to_string(), (key, value, true));
    }

    let runtime_keys = environment
        .keys()
        .filter(|key| is_runtime_root_environment_key(key))
        .cloned()
        .collect::<Vec<_>>();
    for normalized in runtime_keys {
        let Some((key, value, explicit)) = environment.get(&normalized).cloned() else {
            continue;
        };
        let path = PathBuf::from(&value);
        match path.canonicalize().ok().filter(|path| path.is_dir()) {
            Some(path) => {
                environment.insert(
                    normalized,
                    (
                        key,
                        normalized_canonical_path(&path).into_os_string(),
                        explicit,
                    ),
                );
            }
            None if explicit => {
                return Err(ExecutionFailure::without_os_error(
                    ExecutionStage::ResolveRuntime,
                    format!(
                        "explicit runtime environment variable {} points to an unavailable directory: {}",
                        key.to_string_lossy(),
                        path.display()
                    ),
                ));
            }
            None => {
                environment.remove(&normalized);
            }
        }
    }

    Ok(environment
        .into_values()
        .map(|(key, value, _)| (key, value))
        .collect())
}

fn prepend_path_entries(
    environment: &mut BTreeMap<String, (OsString, OsString, bool)>,
    entries: &[PathBuf],
) -> Result<(), ExecutionFailure> {
    if entries.is_empty() {
        return Ok(());
    }
    let current = environment
        .get("PATH")
        .map(|(_, value, _)| value.clone())
        .unwrap_or_default();
    let mut paths = entries.to_vec();
    paths.extend(std::env::split_paths(&current));
    let value = std::env::join_paths(paths.iter()).map_err(|error| {
        ExecutionFailure::without_os_error(
            ExecutionStage::ResolveRuntime,
            format!("failed to prepend workspace tool paths: {error}"),
        )
    })?;
    environment.insert("PATH".to_string(), (OsString::from("PATH"), value, true));
    Ok(())
}

pub(crate) fn configure_command_environment(
    command: &mut Command,
    request: &ExecutionSpec,
    runtime: &ResolvedRuntime,
    config: &LocalSandboxConfig,
) {
    command.env_clear();
    command.envs(runtime.environment.iter().map(|(key, value)| (key, value)));

    if request.stdio != crate::execution_spec::StdioPolicy::Interactive {
        command.envs([
            ("NO_COLOR", "1"),
            ("TERM", "dumb"),
            ("PAGER", "cat"),
            ("GIT_PAGER", "cat"),
            ("GH_PAGER", "cat"),
            ("CI", "1"),
        ]);
    }

    if config.is_enabled() {
        if let Some(home) = runtime.sandbox_home.as_deref() {
            let roaming = home.join("AppData").join("Roaming");
            let local = home.join("AppData").join("Local");
            let config_home = home.join(".config");
            let cache = home.join(".cache");
            let data = home.join(".local").join("share");
            let state = home.join(".local").join("state");
            let temp = home.join("tmp");
            command.env("HOME", home);
            command.env("XDG_CONFIG_HOME", &config_home);
            command.env("XDG_CACHE_HOME", &cache);
            command.env("XDG_DATA_HOME", &data);
            command.env("XDG_STATE_HOME", &state);
            // Package managers must never discover configuration through the
            // interactive user's profile. Besides preventing credential and
            // policy leakage, explicit sandbox-owned paths avoid noisy EPERM
            // warnings when the dedicated account probes an unreadable host
            // pnpm configuration.
            command.env(
                "NPM_CONFIG_USERCONFIG",
                config_home.join("npm").join("npmrc"),
            );
            command.env(
                "NPM_CONFIG_GLOBALCONFIG",
                config_home.join("npm").join("global-npmrc"),
            );
            command.env("NPM_CONFIG_CACHE", cache.join("npm"));
            command.env("PNPM_HOME", data.join("pnpm"));
            if !cfg!(windows)
                || config.effective_windows_backend()
                    == crate::sandbox::WindowsSandboxBackend::DedicatedUser
            {
                command.env("USERPROFILE", home);
                command.env("APPDATA", roaming);
                command.env("LOCALAPPDATA", local);
            }
            command.env("TEMP", &temp);
            command.env("TMP", &temp);
            #[cfg(windows)]
            if config.effective_windows_backend()
                == crate::sandbox::WindowsSandboxBackend::DedicatedUser
            {
                set_windows_home_parts(command, home);
            }
        }
        command.env("OPENTOPIA_SANDBOX", "1");
    }
}

pub(crate) fn environment_keys(runtime: &ResolvedRuntime) -> Vec<String> {
    let mut keys = runtime
        .environment
        .iter()
        .into_iter()
        .map(|(key, _)| key.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    keys.sort_by_key(|key| key.to_ascii_uppercase());
    keys.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    keys
}

fn inherited_environment(request: &ExecutionSpec) -> Vec<(OsString, OsString)> {
    match effective_environment_policy(request) {
        EnvironmentPolicy::Isolated => baseline_environment(),
        EnvironmentPolicy::InheritSanitized => sanitized_environment(),
        EnvironmentPolicy::InteractiveUser => user_environment(),
    }
}

fn runtime_roots_from_environment(
    environment: &[(OsString, OsString)],
) -> Result<Vec<PathBuf>, ExecutionFailure> {
    let mut roots = BTreeSet::new();
    for (key, value) in environment {
        let key = key.to_string_lossy().to_ascii_uppercase();
        if key == "PATH" {
            // PATH is a lookup list, not a declaration that every entry is a
            // dependency of this launch. The resolved executable's parent is
            // already included above; recursively provisioning every PATH
            // directory on Windows is both over-broad and prohibitively slow.
            continue;
        }
        // Keep this to SDK/runtime roots, not arbitrary *_HOME values. For
        // example CARGO_HOME and GRADLE_USER_HOME can contain credentials.
        // A plugin with a less common layout declares its roots explicitly in
        // ExecutionSpec::runtime instead of widening the broker policy.
        if is_runtime_root_environment_key(&key) {
            roots.insert(PathBuf::from(value));
        }
    }
    for root in &roots {
        if !root.is_dir() {
            return Err(ExecutionFailure::without_os_error(
                ExecutionStage::ResolveRuntime,
                format!(
                    "resolved runtime environment contains an unavailable directory: {}",
                    root.display()
                ),
            ));
        }
    }
    Ok(roots
        .into_iter()
        .map(|root| normalized_canonical_path(&root))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn effective_environment_policy(request: &ExecutionSpec) -> EnvironmentPolicy {
    if request.clear_env {
        EnvironmentPolicy::Isolated
    } else {
        request.environment
    }
}

fn baseline_environment() -> Vec<(OsString, OsString)> {
    let baseline = if cfg!(windows) {
        WINDOWS_BASE_ENV
    } else {
        UNIX_BASE_ENV
    };
    baseline
        .iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (OsString::from(key), value)))
        .collect()
}

fn sanitized_environment() -> Vec<(OsString, OsString)> {
    user_environment()
        .into_iter()
        .filter(|(key, _)| !is_sensitive_environment_key(&key.to_string_lossy()))
        .collect()
}

fn user_environment() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(key, _)| {
            let key = key.to_string_lossy();
            !key.is_empty() && !key.contains('=')
        })
        .collect()
}

fn is_sensitive_environment_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    SENSITIVE_CHILD_ENV_KEYS
        .iter()
        .any(|sensitive| key == *sensitive)
        || key.contains("PASSWORD")
        || key.contains("PASSWD")
        || key.contains("SECRET")
        || key.ends_with("_TOKEN")
        || key.ends_with("_API_KEY")
        || key.ends_with("_PRIVATE_KEY")
}

#[cfg(windows)]
fn set_windows_home_parts(command: &mut Command, home: &Path) {
    let value = home.as_os_str().to_string_lossy();
    if value.len() >= 3 && value.as_bytes()[1] == b':' {
        command.env("HOMEDRIVE", &value[..2]);
        command.env("HOMEPATH", &value[2..]);
    }
}

pub(crate) fn configure_stdio(command: &mut Command, request: &ExecutionSpec) {
    match request.stdio {
        crate::execution_spec::StdioPolicy::Null => {
            command.stdin(Stdio::null());
        }
        crate::execution_spec::StdioPolicy::Captured => {
            command.stdin(Stdio::piped());
        }
        crate::execution_spec::StdioPolicy::Interactive => {
            command.stdin(Stdio::inherit());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        is_sensitive_environment_key, normalized_canonical_path, resolve_executable,
        resolve_execution_environment, runtime_roots_from_environment,
    };
    use crate::execution_spec::ExecutionSpec;
    use crate::workspace_execution_capsule::WorkspaceExecutionCapsule;
    use std::ffi::OsString;
    use std::path::Path;

    #[test]
    fn missing_executable_fails_during_runtime_resolution() {
        let environment = resolve_execution_environment(&ExecutionSpec::new("tool"), None)
            .expect("resolve environment");
        let error = resolve_executable(
            "opentopia-command-that-does-not-exist",
            Path::new("."),
            &environment,
        )
        .expect_err("missing executable must fail");
        assert_eq!(
            error.stage,
            crate::execution_spec::ExecutionStage::ResolveRuntime
        );
    }

    #[test]
    fn sanitized_environment_keeps_toolchain_configuration_and_drops_credentials() {
        assert!(is_sensitive_environment_key("SERVICE_API_KEY"));
        assert!(is_sensitive_environment_key("database_password"));
        assert!(is_sensitive_environment_key("AWS_SESSION_TOKEN"));
        assert!(is_sensitive_environment_key("CARGO_HOME"));
        assert!(!is_sensitive_environment_key("JAVA_HOME"));

        let environment = resolve_execution_environment(&ExecutionSpec::new("tool"), None)
            .expect("resolve environment");
        let keys = environment
            .iter()
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(keys.iter().any(|key| key.eq_ignore_ascii_case("PATH")));
        assert!(!keys
            .iter()
            .any(|key| key.eq_ignore_ascii_case("OPENAI_API_KEY")));
    }

    #[test]
    fn workspace_capsule_environment_is_projected_for_shells() {
        let root = std::env::temp_dir().join(format!(
            "opentopia-execution-capsule-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create capsule workspace");
        let capsule = WorkspaceExecutionCapsule::discover(&root);
        let expected_root = normalized_canonical_path(&root);

        let shell = resolve_execution_environment(
            &ExecutionSpec::shell("echo ok").env_clear(),
            Some(&capsule),
        )
        .expect("resolve shell capsule environment");
        assert!(shell.iter().any(|(key, value)| {
            key.to_string_lossy()
                .eq_ignore_ascii_case("GIT_CONFIG_VALUE_0")
                && value
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&expected_root.to_string_lossy())
        }));

        let direct = resolve_execution_environment(&ExecutionSpec::new("tool").env_clear(), None)
            .expect("resolve direct execution environment");
        assert!(!direct.iter().any(|(key, _)| key
            .to_string_lossy()
            .eq_ignore_ascii_case("GIT_CONFIG_VALUE_0")));

        std::fs::remove_dir_all(root).expect("remove capsule workspace");
    }

    #[test]
    fn runtime_roots_are_inferred_without_tool_specific_adapters() {
        let root = std::env::current_dir().expect("current directory");
        let canonical_root = normalized_canonical_path(&root);
        let path = std::env::join_paths([root.clone()]).expect("join PATH fixture");
        let roots = runtime_roots_from_environment(&[
            (OsString::from("PATH"), path),
            (OsString::from("JAVA_HOME"), root.as_os_str().to_os_string()),
            (
                OsString::from("SERVICE_TOKEN"),
                root.as_os_str().to_os_string(),
            ),
        ])
        .expect("runtime roots");
        assert_eq!(roots, vec![canonical_root]);
    }

    #[test]
    fn unavailable_path_entries_are_removed_without_becoming_sandbox_grants() {
        let root = std::env::current_dir()
            .expect("current directory")
            .canonicalize()
            .expect("canonical root");
        let root = normalized_canonical_path(&root);
        let missing = std::env::temp_dir().join("opentopia-missing-runtime-path-entry");
        let path = std::env::join_paths([missing, root.clone()]).expect("join PATH fixture");
        let request = ExecutionSpec::new("tool").env_clear().env("PATH", path);

        let environment =
            resolve_execution_environment(&request, None).expect("resolve environment");
        let resolved_path = environment
            .iter()
            .find(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case("PATH"))
            .map(|(_, value)| value)
            .expect("resolved PATH");
        assert_eq!(
            std::env::split_paths(resolved_path).collect::<Vec<_>>(),
            vec![root.clone()]
        );
        assert!(runtime_roots_from_environment(&environment)
            .expect("runtime roots")
            .is_empty());
    }

    #[test]
    fn resolved_path_preserves_lookup_precedence_and_duplicates() {
        let fixture = std::env::temp_dir().join(format!(
            "opentopia-runtime-path-order-{}",
            std::process::id()
        ));
        let first = fixture.join("z-first");
        let second = fixture.join("a-second");
        std::fs::create_dir_all(&first).expect("create first PATH directory");
        std::fs::create_dir_all(&second).expect("create second PATH directory");
        let first = normalized_canonical_path(&first);
        let second = normalized_canonical_path(&second);
        let path = std::env::join_paths([first.clone(), second.clone(), first.clone()])
            .expect("join ordered PATH fixture");
        let request = ExecutionSpec::new("tool").env_clear().env("PATH", path);

        let environment =
            resolve_execution_environment(&request, None).expect("resolve environment");
        let resolved_path = environment
            .iter()
            .find(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case("PATH"))
            .map(|(_, value)| value)
            .expect("resolved PATH");
        assert_eq!(
            std::env::split_paths(resolved_path).collect::<Vec<_>>(),
            vec![first.clone(), second, first]
        );

        std::fs::remove_dir_all(fixture).expect("remove PATH fixture");
    }

    #[test]
    fn explicit_unavailable_runtime_home_fails_during_resolution() {
        let missing = std::env::temp_dir().join("opentopia-missing-java-home");
        let error = resolve_execution_environment(
            &ExecutionSpec::new("tool")
                .env_clear()
                .env("JAVA_HOME", missing.as_os_str()),
            None,
        )
        .expect_err("explicit runtime roots must be valid");

        assert_eq!(
            error.stage,
            crate::execution_spec::ExecutionStage::ResolveRuntime
        );
        assert!(error.message.contains("JAVA_HOME"));
    }
}
