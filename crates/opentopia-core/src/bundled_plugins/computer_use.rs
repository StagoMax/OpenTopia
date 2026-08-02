use super::{BundledPluginFile, BundledPluginMetadata, BundledPluginPackage, BundledPluginTrust};

const MANIFEST: &[u8] =
    include_bytes!("../../bundled-plugins/computer-use/.codex-plugin/plugin.json");
const CONFIGURATION_SCHEMA: &[u8] =
    include_bytes!("../../bundled-plugins/computer-use/configuration.schema.json");

const FILES: &[BundledPluginFile] = &[
    BundledPluginFile {
        relative_path: ".codex-plugin/plugin.json",
        contents: MANIFEST,
    },
    BundledPluginFile {
        relative_path: "configuration.schema.json",
        contents: CONFIGURATION_SCHEMA,
    },
];

pub(super) const PACKAGE: BundledPluginPackage = BundledPluginPackage {
    metadata: BundledPluginMetadata {
        name: "computer-use",
        version: "1.0.0",
        trust: BundledPluginTrust::TrustedDriver,
        default_enabled: false,
        native_capabilities: &["computer"],
    },
    files: FILES,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn package_owns_driver_trust_and_default_installation_metadata() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid plugin manifest");

        assert_eq!(PACKAGE.metadata.name, manifest["name"]);
        assert_eq!(PACKAGE.metadata.version, manifest["version"]);
        assert_eq!(PACKAGE.metadata.trust, BundledPluginTrust::TrustedDriver);
        assert!(!PACKAGE.metadata.default_enabled);
        assert_eq!(PACKAGE.metadata.native_capabilities, &["computer"]);
        assert!(manifest.get("trust").is_none());
        assert!(manifest.get("official").is_none());
        assert!(manifest["opentopia"].get("trust").is_none());
        assert!(manifest["opentopia"].get("official").is_none());
    }

    #[test]
    fn manifest_attributes_the_tool_without_self_registering_a_driver() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid plugin manifest");
        let opentopia = &manifest["opentopia"];
        let contributes = &opentopia["contributes"];
        let tool = &contributes["nativeTools"][0];

        assert_eq!(opentopia["apiVersion"], "1");
        assert_eq!(tool["id"], "computer");
        assert_eq!(tool["runtime"], "builtin:computer");
        assert_eq!(tool["driverCapability"], "computer.driver.v1");
        assert!(contributes.get("computerDrivers").is_none());
        assert!(opentopia["requires"]["hostCapabilities"]
            .as_array()
            .expect("host capability list")
            .iter()
            .any(|capability| capability == "policy.approval.v1"));
        assert_eq!(
            opentopia["permissions"]["desktop"],
            serde_json::json!(["window:enumerate", "window:capture", "window:input"])
        );
        assert_eq!(
            opentopia["configuration"]["schema"],
            "./configuration.schema.json"
        );

        let schema: Value =
            serde_json::from_slice(CONFIGURATION_SCHEMA).expect("valid configuration schema");
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
    }
}
