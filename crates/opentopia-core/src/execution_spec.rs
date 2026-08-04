use crate::sandbox::NetworkPolicy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

/// Standard-input behavior for a launched process.
///
/// Non-interactive execution defaults to `Null`; callers must opt in to a pipe
/// or an interactive terminal. This prevents background tools from silently
/// inheriting the server's standard input and waiting forever for a prompt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StdioPolicy {
    #[default]
    Null,
    Captured,
    Interactive,
}

/// How much of the host environment a command is allowed to inherit.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentPolicy {
    /// Preserve the historical behavior while removing known secret values.
    #[default]
    InheritSanitized,
    /// Start from a small platform runtime baseline and apply explicit values.
    Isolated,
    /// Inherit the user's environment for an explicitly interactive session.
    InteractiveUser,
}

/// Runtime roots needed in addition to the executable itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRequirements {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub read_roots: Vec<PathBuf>,
}

/// Per-command filesystem and network requirements.
///
/// These are requirements, not grants. The execution environment rejects a
/// request when its sandbox policy cannot satisfy them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRequirements {
    #[serde(default)]
    pub read_paths: Vec<PathBuf>,
    #[serde(default)]
    pub write_paths: Vec<PathBuf>,
    #[serde(default)]
    pub deny_read_paths: Vec<PathBuf>,
    #[serde(default)]
    pub deny_write_paths: Vec<PathBuf>,
    #[serde(default)]
    pub network: Option<NetworkPolicy>,
}

#[derive(Debug, Clone)]
pub struct ExecutionSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub stdin: Option<Vec<u8>>,
    pub stdio: StdioPolicy,
    pub environment: EnvironmentPolicy,
    /// Retained for source compatibility with the old request builder.
    pub clear_env: bool,
    pub env: HashMap<OsString, OsString>,
    pub runtime: RuntimeRequirements,
    pub requirements: ExecutionRequirements,
}

impl ExecutionSpec {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            stdin: None,
            stdio: StdioPolicy::Null,
            environment: EnvironmentPolicy::InheritSanitized,
            clear_env: false,
            env: HashMap::new(),
            runtime: RuntimeRequirements::default(),
            requirements: ExecutionRequirements::default(),
        }
    }

    pub fn shell(command: impl Into<String>) -> Self {
        let command = command.into();
        if cfg!(windows) {
            Self::new("powershell.exe")
                .arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-Command")
                .arg(command)
        } else {
            Self::new("sh").arg("-lc").arg(command)
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn stdin(mut self, stdin: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(stdin.into());
        self.stdio = StdioPolicy::Captured;
        self
    }

    pub fn stdin_null(mut self) -> Self {
        self.stdin = None;
        self.stdio = StdioPolicy::Null;
        self
    }

    pub fn interactive(mut self) -> Self {
        self.stdin = None;
        self.stdio = StdioPolicy::Interactive;
        self.environment = EnvironmentPolicy::InteractiveUser;
        self
    }

    pub fn env_clear(mut self) -> Self {
        self.clear_env = true;
        self.environment = EnvironmentPolicy::Isolated;
        self
    }

    pub fn environment_policy(mut self, policy: EnvironmentPolicy) -> Self {
        self.clear_env = policy == EnvironmentPolicy::Isolated;
        self.environment = policy;
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn envs<K, V>(mut self, variables: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<OsString>,
        V: Into<OsString>,
    {
        self.env.extend(
            variables
                .into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        self
    }

    pub fn runtime(mut self, name: impl Into<String>, read_roots: Vec<PathBuf>) -> Self {
        self.runtime = RuntimeRequirements {
            name: Some(name.into()),
            read_roots,
        };
        self
    }

    pub fn requirements(mut self, requirements: ExecutionRequirements) -> Self {
        self.requirements = requirements;
        self
    }
}

/// Backwards-compatible name used by the existing execution APIs.
pub type ExecRequest = ExecutionSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifecyclePolicy {
    pub startup_timeout: Duration,
    pub execution_timeout: Duration,
    pub termination_timeout: Duration,
}

impl Default for LifecyclePolicy {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::from_secs(15),
            execution_timeout: Duration::from_secs(30),
            termination_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStage {
    ResolveRuntime,
    ValidatePolicy,
    PrepareSandbox,
    Spawn,
    Wait,
    Terminate,
    CollectOutput,
}

#[derive(Debug, thiserror::Error)]
#[error("execution failed during {stage:?}: {message}")]
pub struct ExecutionFailure {
    pub stage: ExecutionStage,
    pub message: String,
    pub os_error: Option<i32>,
}

impl ExecutionFailure {
    pub fn new(stage: ExecutionStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
            os_error: std::io::Error::last_os_error().raw_os_error(),
        }
    }

    pub fn without_os_error(stage: ExecutionStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
            os_error: None,
        }
    }

    pub fn from_io(
        stage: ExecutionStage,
        message: impl Into<String>,
        error: &std::io::Error,
    ) -> Self {
        Self {
            stage,
            message: message.into(),
            os_error: error.raw_os_error(),
        }
    }
}
