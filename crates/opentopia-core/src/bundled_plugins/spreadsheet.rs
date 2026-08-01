use super::{BundledPluginFile, BundledPluginMetadata, BundledPluginPackage, BundledPluginTrust};

const MANIFEST: &[u8] =
    include_bytes!("../../bundled-plugins/spreadsheet/.codex-plugin/plugin.json");
const CONFIGURATION_SCHEMA: &[u8] =
    include_bytes!("../../bundled-plugins/spreadsheet/configuration.schema.json");

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
        name: "spreadsheet",
        version: "1.0.0",
        trust: BundledPluginTrust::Official,
        default_enabled: true,
        native_capabilities: &["spreadsheet"],
    },
    files: FILES,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{SpreadsheetTool, Tool};
    use serde_json::Value;

    #[test]
    fn package_and_manifest_keep_host_owned_trust_out_of_plugin_data() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid plugin manifest");

        assert_eq!(PACKAGE.metadata.name, manifest["name"]);
        assert_eq!(PACKAGE.metadata.version, manifest["version"]);
        assert_eq!(PACKAGE.metadata.trust, BundledPluginTrust::Official);
        assert!(PACKAGE.metadata.default_enabled);
        assert_eq!(PACKAGE.metadata.native_capabilities, &["spreadsheet"]);
        assert!(manifest.get("trust").is_none());
        assert!(manifest.get("official").is_none());
        assert!(manifest["opentopia"].get("trust").is_none());
        assert!(manifest["opentopia"].get("official").is_none());
    }

    #[test]
    fn manifest_registers_the_existing_tool_and_xlsx_previewer() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid plugin manifest");
        let opentopia = &manifest["opentopia"];
        let tool = SpreadsheetTool;

        assert_eq!(opentopia["apiVersion"], "1");
        assert_eq!(
            opentopia["contributes"]["nativeTools"][0]["id"],
            tool.name()
        );
        assert_eq!(
            tool.schema()["properties"]["action"]["enum"],
            serde_json::json!(["inspect", "list_sheets", "read_range", "write"])
        );
        assert_eq!(
            opentopia["contributes"]["previewers"][0]["mediaTypes"][0],
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
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
