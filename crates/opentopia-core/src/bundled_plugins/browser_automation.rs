use super::{BundledPluginFile, BundledPluginMetadata, BundledPluginPackage, BundledPluginTrust};

const MANIFEST: &[u8] =
    include_bytes!("../../bundled-plugins/browser-automation/.codex-plugin/plugin.json");
const CONFIGURATION_SCHEMA: &[u8] =
    include_bytes!("../../bundled-plugins/browser-automation/configuration.schema.json");

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
        name: "browser-automation",
        version: "1.0.0",
        trust: BundledPluginTrust::Privileged,
        default_enabled: true,
        native_capabilities: &["browser"],
    },
    files: FILES,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn package_owns_trust_and_default_installation_metadata() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid plugin manifest");

        assert_eq!(PACKAGE.metadata.name, manifest["name"]);
        assert_eq!(PACKAGE.metadata.version, manifest["version"]);
        assert_eq!(PACKAGE.metadata.trust, BundledPluginTrust::Privileged);
        assert!(PACKAGE.metadata.default_enabled);
        assert_eq!(PACKAGE.metadata.native_capabilities, &["browser"]);
        assert!(manifest.get("trust").is_none());
        assert!(manifest.get("official").is_none());
        assert!(manifest["opentopia"].get("trust").is_none());
        assert!(manifest["opentopia"].get("official").is_none());
    }

    #[test]
    fn manifest_attributes_the_existing_browser_tool_and_runtime() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid plugin manifest");
        let opentopia = &manifest["opentopia"];
        let tool = &opentopia["contributes"]["nativeTools"][0];

        assert_eq!(opentopia["apiVersion"], "1");
        assert_eq!(tool["id"], "browser");
        assert_eq!(tool["runtime"], "builtin:browser");
        assert_eq!(tool["driverCapability"], "browser.runtime.v1");
        assert_eq!(
            opentopia["permissions"]["network"][0],
            "user-approved-domains"
        );
        assert_eq!(
            opentopia["permissions"]["desktop"][0],
            "browser:visible-surface"
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
