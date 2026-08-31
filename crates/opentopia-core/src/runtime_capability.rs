//! Host-owned child-process runtime capabilities.
//!
//! Runtime providers register executable environment, readable roots, and
//! managed permission roots here. Shell execution consumes the resulting
//! projections without depending on tool-specific schemas or parsing commands.

use crate::office_runtime::{OfficePythonRuntime, OfficeRuntime, OfficeRuntimeSource};
use std::ffi::OsString;
use std::path::PathBuf;

pub(crate) const OFFICE_PYTHON_EXECUTABLE_ENV: &str = "OPENTOPIA_OFFICE_PYTHON";

#[derive(Debug, Clone, Default)]
pub(crate) struct ChildProcessRuntimeCapability {
    pub environment: Vec<(OsString, OsString)>,
    pub path_entries: Vec<PathBuf>,
    pub read_roots: Vec<PathBuf>,
    pub managed_runtime_roots: Vec<PathBuf>,
}

impl ChildProcessRuntimeCapability {
    pub(crate) fn with_environment(
        mut self,
        key: impl Into<OsString>,
        value: impl Into<OsString>,
    ) -> Self {
        self.environment.push((key.into(), value.into()));
        self
    }

    pub(crate) fn with_path_entry(mut self, path: PathBuf) -> Self {
        self.path_entries.push(path);
        self
    }

    pub(crate) fn with_read_root(mut self, path: PathBuf) -> Self {
        self.read_roots.push(path);
        self
    }

    pub(crate) fn with_managed_runtime_root(mut self, path: PathBuf) -> Self {
        self.managed_runtime_roots.push(path);
        self
    }
}

pub(crate) fn registered_child_process_runtimes() -> Vec<ChildProcessRuntimeCapability> {
    OfficeRuntime::shared()
        .status()
        .runtime
        .as_ref()
        .map(office_python_capability)
        .into_iter()
        .collect()
}

fn office_python_capability(runtime: &OfficePythonRuntime) -> ChildProcessRuntimeCapability {
    let capability = ChildProcessRuntimeCapability::default()
        .with_environment(OFFICE_PYTHON_EXECUTABLE_ENV, runtime.executable.as_os_str())
        .with_read_root(runtime.root.clone());
    if runtime.source == OfficeRuntimeSource::Managed {
        capability.with_managed_runtime_root(runtime.root.clone())
    } else {
        capability
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn office_python_uses_one_registered_runtime_projection() {
        let runtime = OfficePythonRuntime {
            executable: PathBuf::from("runtime/python/python.exe"),
            root: PathBuf::from("runtime"),
            runtime_version: "office-test".to_string(),
            python_version: "3.12.14".to_string(),
            openpyxl_version: "3.1.5".to_string(),
            source: OfficeRuntimeSource::Managed,
        };

        let capability = office_python_capability(&runtime);
        assert_eq!(
            capability.environment,
            [(
                OsString::from(OFFICE_PYTHON_EXECUTABLE_ENV),
                runtime.executable.into_os_string()
            )]
        );
        assert_eq!(capability.read_roots, [runtime.root.clone()]);
        assert_eq!(capability.managed_runtime_roots, [runtime.root]);
        assert!(capability.path_entries.is_empty());
    }
}
