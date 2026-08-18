mod command;
mod contract;
mod descriptor;
mod path_policy;
mod windows_backend;

#[cfg(all(test, windows))]
pub(crate) use command::dedicated_user_credentials_are_installed_for_tests;
pub use command::{
    build_local_sandbox_command, build_local_sandbox_command_for_platform,
    build_local_sandbox_command_for_platform_with_options,
    build_local_sandbox_command_with_options, sandbox_permission_profile,
};
pub use contract::{
    ExecutionEnvironmentKind, LocalSandboxConfig, NetworkPolicy, OsSandboxMode, OsSandboxPlatform,
    SandboxBackendCapabilities, SandboxCommandPlan, SandboxCommandStatus, SandboxLaunchOptions,
    SandboxLifecycle, SandboxMode, SandboxPreparationPlan, WindowsSandboxBackend,
    WindowsSandboxSetupComponents, WindowsSandboxSetupState, WindowsSandboxSetupStatus,
};
pub use descriptor::SandboxDescriptor;
pub use path_policy::is_protected_metadata_path;
pub use windows_backend::{
    remove_windows_sandbox, setup_windows_sandbox, windows_sandbox_setup_status,
};

#[cfg(test)]
mod tests;
