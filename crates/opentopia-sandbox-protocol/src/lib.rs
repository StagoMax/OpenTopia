use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SANDBOX_PROTOCOL_SCHEMA: &str = "ai.opentopia.sandbox.protocol";
pub const SANDBOX_PROTOCOL_VERSION: u32 = 2;
pub const SANDBOX_SETUP_STATUS_SCHEMA: &str = "ai.opentopia.sandbox.setup-status";
pub const SANDBOX_SETUP_STATUS_VERSION: u32 = 2;
pub const REQUIRED_SANDBOX_FEATURES: &[&str] = &[
    "error.envelope.v1",
    "run.backend",
    "run.denied_read_paths",
    "run.filesystem_capabilities.v1",
    "run.interactive",
    "run.protected_roots",
    "run.resource_limits",
    "run.runtime_roots",
    "setup.lifecycle.v1",
];

/// Filesystem access requested by one sandbox launch. Read access and write
/// access deliberately use different authorization models: an existing-only
/// read dependency must already be available to the sandbox identity, while a
/// managed read root may be provisioned by OpenTopia. Writes are always gated
/// by a per-scope restricting SID in addition to the sandbox user's normal ACLs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FilesystemCapabilities {
    pub read_execute: Vec<ReadExecuteCapability>,
    pub write: Vec<PathBuf>,
    pub deny_read: Vec<PathBuf>,
    pub deny_write: Vec<PathBuf>,
    pub allow_protected_write: Vec<PathBuf>,
    /// Managed per-command home used for profile, configuration, and temporary
    /// files. It is part of the write capability, rather than the dedicated
    /// Windows account's shared profile.
    #[serde(default)]
    pub runtime_home: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadExecuteCapability {
    pub path: PathBuf,
    pub provisioning: ReadProvisioning,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadProvisioning {
    /// OpenTopia owns the policy scope and may add a dedicated-user read ACE
    /// when the account does not already have effective access.
    Managed,
    /// The path is an external dependency such as a PATH/SDK runtime. Its host
    /// ACL is never rewritten as part of command startup.
    ExistingOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxSetupState {
    NotConfigured,
    Ready,
    Degraded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSetupComponents {
    pub credentials: bool,
    pub offline_identity: bool,
    pub online_identity: bool,
    pub offline_network_policy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSetupStatus {
    pub schema: String,
    pub status_version: u32,
    pub state: SandboxSetupState,
    pub state_dir: String,
    pub components: SandboxSetupComponents,
    pub issues: Vec<String>,
}

impl SandboxSetupStatus {
    pub fn current(
        state: SandboxSetupState,
        state_dir: impl Into<String>,
        components: SandboxSetupComponents,
        issues: Vec<String>,
    ) -> Self {
        Self {
            schema: SANDBOX_SETUP_STATUS_SCHEMA.to_string(),
            status_version: SANDBOX_SETUP_STATUS_VERSION,
            state,
            state_dir: state_dir.into(),
            components,
            issues,
        }
    }

    pub fn is_ready(&self) -> bool {
        self.state == SandboxSetupState::Ready
    }

    pub fn compatibility_error(&self) -> Option<String> {
        if self.schema != SANDBOX_SETUP_STATUS_SCHEMA {
            return Some(format!(
                "unexpected setup-status schema '{}'; expected '{}'",
                self.schema, SANDBOX_SETUP_STATUS_SCHEMA
            ));
        }
        (self.status_version != SANDBOX_SETUP_STATUS_VERSION).then(|| {
            format!(
                "setup-status version {} is incompatible with required version {}",
                self.status_version, SANDBOX_SETUP_STATUS_VERSION
            )
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxProtocolInfo {
    pub schema: String,
    pub protocol_version: u32,
    pub helper_version: String,
    pub features: Vec<String>,
}

impl SandboxProtocolInfo {
    pub fn current(helper_version: impl Into<String>) -> Self {
        Self {
            schema: SANDBOX_PROTOCOL_SCHEMA.to_string(),
            protocol_version: SANDBOX_PROTOCOL_VERSION,
            helper_version: helper_version.into(),
            features: REQUIRED_SANDBOX_FEATURES
                .iter()
                .map(|feature| (*feature).to_string())
                .collect(),
        }
    }

    pub fn compatibility_error(&self) -> Option<String> {
        if self.schema != SANDBOX_PROTOCOL_SCHEMA {
            return Some(format!(
                "unexpected protocol schema '{}'; expected '{}'",
                self.schema, SANDBOX_PROTOCOL_SCHEMA
            ));
        }
        if self.protocol_version != SANDBOX_PROTOCOL_VERSION {
            return Some(format!(
                "protocol version {} is incompatible with required version {}",
                self.protocol_version, SANDBOX_PROTOCOL_VERSION
            ));
        }
        let missing = REQUIRED_SANDBOX_FEATURES
            .iter()
            .filter(|feature| !self.features.iter().any(|value| value == **feature))
            .copied()
            .collect::<Vec<_>>();
        (!missing.is_empty()).then(|| format!("missing required features: {}", missing.join(", ")))
    }

    pub fn is_compatible(&self) -> bool {
        self.compatibility_error().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_protocol_is_self_compatible() {
        let info = SandboxProtocolInfo::current("test");
        assert!(info.is_compatible());
    }

    #[test]
    fn missing_feature_is_rejected() {
        let mut info = SandboxProtocolInfo::current("test");
        info.features.retain(|feature| feature != "run.backend");
        assert_eq!(
            info.compatibility_error().as_deref(),
            Some("missing required features: run.backend")
        );
    }

    #[test]
    fn current_setup_status_is_compatible() {
        let status = SandboxSetupStatus::current(
            SandboxSetupState::NotConfigured,
            "C:/state",
            SandboxSetupComponents::default(),
            Vec::new(),
        );
        assert_eq!(status.compatibility_error(), None);
        assert!(!status.is_ready());
    }

    #[test]
    fn filesystem_capabilities_round_trip_with_explicit_provisioning() {
        let capabilities = FilesystemCapabilities {
            read_execute: vec![ReadExecuteCapability {
                path: PathBuf::from(r"J:\Python311"),
                provisioning: ReadProvisioning::ExistingOnly,
            }],
            write: vec![PathBuf::from(r"J:\workspace")],
            runtime_home: Some(PathBuf::from(r"J:\sandbox-home")),
            ..Default::default()
        };
        let encoded = serde_json::to_vec(&capabilities).expect("serialize capabilities");
        let decoded = serde_json::from_slice(&encoded).expect("deserialize capabilities");
        assert_eq!(decoded, capabilities);
    }
}
