use crate::powershell_runtime::current_shell_runtime;
use crate::sandbox::NetworkPolicy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellDialect {
    PowerShell7,
    WindowsPowerShell51,
    PosixSh,
}

impl ShellDialect {
    pub fn current() -> Self {
        current_shell_runtime().dialect
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::PowerShell7 => "powershell_7",
            Self::WindowsPowerShell51 => "windows_powershell_5_1",
            Self::PosixSh => "posix_sh",
        }
    }

    pub fn model_guidance(self) -> &'static str {
        match self {
            Self::PowerShell7 => {
                "Shell commands use PowerShell 7 on Windows. Use PowerShell syntax, not Bash; `&&`/`||` are supported, `Select-Object` replaces `head`/`tail`, and `$null` discards output. UTF-8 script files are supported with or without a BOM."
            }
            Self::WindowsPowerShell51 => {
                "Shell commands use Windows PowerShell 5.1, not Bash or PowerShell 7. Use `;` or explicit `$LASTEXITCODE` checks instead of `&&`/`||`, `Select-Object -First/-Last` instead of `head`/`tail`, and `$null` for discarded output."
            }
            Self::PosixSh => {
                "Shell commands use POSIX `sh`; do not use PowerShell cmdlets or `$env:` syntax."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCompatibilityError {
    pub code: &'static str,
    pub message: String,
}

pub fn shell_command_compatibility_error(command: &str) -> Option<ShellCompatibilityError> {
    if ShellDialect::current() != ShellDialect::WindowsPowerShell51 {
        return None;
    }
    let operator = unsupported_windows_powershell_operator(command)?;
    Some(ShellCompatibilityError {
        code: "shell_dialect_mismatch",
        message: format!(
            "The active shell is Windows PowerShell 5.1, where unquoted `{operator}` is invalid. Use separate shell calls, `;`, or an explicit `$LASTEXITCODE` check. Use `Select-Object -First/-Last` instead of `head`/`tail`, and redirect discarded output to `$null`."
        ),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerShellLexState {
    Code,
    SingleQuoted,
    DoubleQuoted,
    SingleQuotedHereString,
    DoubleQuotedHereString,
    LineComment,
    BlockComment,
}

fn unsupported_windows_powershell_operator(command: &str) -> Option<&'static str> {
    let chars = command.chars().collect::<Vec<_>>();
    let mut state = PowerShellLexState::Code;
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        let next = chars.get(index + 1).copied();
        match state {
            PowerShellLexState::Code => {
                if current == '-' && next == Some('-') && chars.get(index + 2) == Some(&'%') {
                    return None;
                }
                if current == '<' && next == Some('#') {
                    state = PowerShellLexState::BlockComment;
                    index += 2;
                    continue;
                }
                if current == '#' {
                    state = PowerShellLexState::LineComment;
                    index += 1;
                    continue;
                }
                if current == '@' && next == Some('\'') {
                    state = PowerShellLexState::SingleQuotedHereString;
                    index += 2;
                    continue;
                }
                if current == '@' && next == Some('"') {
                    state = PowerShellLexState::DoubleQuotedHereString;
                    index += 2;
                    continue;
                }
                if current == '\'' {
                    state = PowerShellLexState::SingleQuoted;
                    index += 1;
                    continue;
                }
                if current == '"' {
                    state = PowerShellLexState::DoubleQuoted;
                    index += 1;
                    continue;
                }
                if current == '`' {
                    index += 2;
                    continue;
                }
                if current == '&' && next == Some('&') {
                    return Some("&&");
                }
                if current == '|' && next == Some('|') {
                    return Some("||");
                }
            }
            PowerShellLexState::SingleQuoted => {
                if current == '\'' {
                    if next == Some('\'') {
                        index += 2;
                        continue;
                    }
                    state = PowerShellLexState::Code;
                }
            }
            PowerShellLexState::DoubleQuoted => {
                if current == '`' {
                    index += 2;
                    continue;
                }
                if current == '"' {
                    state = PowerShellLexState::Code;
                }
            }
            PowerShellLexState::SingleQuotedHereString => {
                if is_here_string_terminator(&chars, index, '\'') {
                    state = PowerShellLexState::Code;
                    index += 2;
                    continue;
                }
            }
            PowerShellLexState::DoubleQuotedHereString => {
                if is_here_string_terminator(&chars, index, '"') {
                    state = PowerShellLexState::Code;
                    index += 2;
                    continue;
                }
            }
            PowerShellLexState::LineComment => {
                if current == '\n' {
                    state = PowerShellLexState::Code;
                }
            }
            PowerShellLexState::BlockComment => {
                if current == '#' && next == Some('>') {
                    state = PowerShellLexState::Code;
                    index += 2;
                    continue;
                }
            }
        }
        index += 1;
    }
    None
}

fn is_here_string_terminator(chars: &[char], index: usize, quote: char) -> bool {
    chars.get(index) == Some(&quote)
        && chars.get(index + 1) == Some(&'@')
        && (index == 0 || chars.get(index.wrapping_sub(1)) == Some(&'\n'))
}

fn powershell_wrapper(command: &str) -> String {
    format!(
        "$__otUtf8 = New-Object System.Text.UTF8Encoding($false); \
         [Console]::InputEncoding = $__otUtf8; \
         [Console]::OutputEncoding = $__otUtf8; \
         $OutputEncoding = $__otUtf8; \
         $ErrorActionPreference = 'Stop'; \
         {command}"
    )
}

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

/// Host-provided capabilities that must be projected into an execution.
///
/// Capabilities are intentionally separate from [`RuntimeRequirements`]. A
/// runtime describes the executable being launched, while a capability
/// describes workspace-scoped tools and policy that descendants of that
/// executable also need (for example Git policy inside PowerShell).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionCapability {
    WorkspaceShell,
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
    pub capabilities: Vec<ExecutionCapability>,
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
            capabilities: Vec::new(),
        }
    }

    pub fn shell(command: impl Into<String>) -> Self {
        let command = command.into();
        if cfg!(windows) {
            let runtime = current_shell_runtime();
            Self::new(runtime.program.to_string_lossy().into_owned())
                .arg("-NoProfile")
                .arg("-NonInteractive")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-Command")
                .arg(powershell_wrapper(&command))
                .runtime("powershell", runtime.runtime_read_roots())
                .capability(ExecutionCapability::WorkspaceShell)
        } else {
            Self::new("sh")
                .arg("-lc")
                .arg(command)
                .capability(ExecutionCapability::WorkspaceShell)
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

    pub fn capability(mut self, capability: ExecutionCapability) -> Self {
        if !self.capabilities.contains(&capability) {
            self.capabilities.push(capability);
        }
        self
    }

    pub fn requires_capability(&self, capability: ExecutionCapability) -> bool {
        self.capabilities.contains(&capability)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_dialect_matches_the_platform() {
        let current = ShellDialect::current();
        if cfg!(windows) {
            assert!(matches!(
                current,
                ShellDialect::PowerShell7 | ShellDialect::WindowsPowerShell51
            ));
        } else {
            assert_eq!(current, ShellDialect::PosixSh);
        }
        assert!(!current.model_guidance().is_empty());
    }

    #[test]
    fn windows_powershell_operator_scan_ignores_literals_comments_and_here_strings() {
        assert_eq!(
            unsupported_windows_powershell_operator("git status && git log"),
            Some("&&")
        );
        assert_eq!(
            unsupported_windows_powershell_operator("git status || git log"),
            Some("||")
        );
        assert_eq!(
            unsupported_windows_powershell_operator("Write-Output 'a && b'"),
            None
        );
        assert_eq!(
            unsupported_windows_powershell_operator("Write-Output \"a || b\" # && ignored"),
            None
        );
        assert_eq!(
            unsupported_windows_powershell_operator("@'\na && b\n'@\nWrite-Output ok"),
            None
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_shell_initializes_utf8_before_user_input() {
        let command = format!("Write-Output '{}{}'", '\u{4e2d}', '\u{6587}');
        let request = ExecutionSpec::shell(&command);
        assert!(matches!(
            ShellDialect::current(),
            ShellDialect::PowerShell7 | ShellDialect::WindowsPowerShell51
        ));
        let wrapper = request.args.last().expect("PowerShell wrapper");
        assert!(wrapper.contains("[Console]::OutputEncoding = $__otUtf8"));
        assert!(wrapper.ends_with(&command));
        assert!(
            wrapper.find("[Console]::OutputEncoding").unwrap() < wrapper.find(&command).unwrap()
        );
    }
}
