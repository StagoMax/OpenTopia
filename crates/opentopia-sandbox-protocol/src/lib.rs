use serde::{Deserialize, Serialize};

pub const SANDBOX_PROTOCOL_SCHEMA: &str = "ai.opentopia.sandbox.protocol";
pub const SANDBOX_PROTOCOL_VERSION: u32 = 1;
pub const REQUIRED_SANDBOX_FEATURES: &[&str] = &[
    "error.envelope.v1",
    "run.backend",
    "run.denied_read_paths",
    "run.interactive",
    "run.protected_roots",
    "run.resource_limits",
    "run.runtime_roots",
];

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
}
