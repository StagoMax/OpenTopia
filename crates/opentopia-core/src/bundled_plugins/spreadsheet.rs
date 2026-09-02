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
        version: "2.0.0",
        trust: BundledPluginTrust::Official,
        default_enabled: true,
        native_capabilities: &[
            "spreadsheet",
            "spreadsheet_inspect",
            "spreadsheet_describe",
            "spreadsheet_execute",
        ],
    },
    files: FILES,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{
        SpreadsheetDescribeTool, SpreadsheetExecuteTool, SpreadsheetInspectTool, SpreadsheetTool,
        Tool,
    };
    use serde_json::Value;

    fn action_names(schema: &Value) -> Vec<String> {
        schema["oneOf"]
            .as_array()
            .expect("action branches")
            .iter()
            .map(|branch| {
                branch["properties"]["action"]["enum"][0]
                    .as_str()
                    .expect("action name")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn package_and_manifest_keep_host_owned_trust_out_of_plugin_data() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid plugin manifest");

        assert_eq!(PACKAGE.metadata.name, manifest["name"]);
        assert_eq!(PACKAGE.metadata.version, manifest["version"]);
        assert_eq!(PACKAGE.metadata.trust, BundledPluginTrust::Official);
        assert!(PACKAGE.metadata.default_enabled);
        assert_eq!(
            PACKAGE.metadata.native_capabilities,
            &[
                "spreadsheet",
                "spreadsheet_inspect",
                "spreadsheet_describe",
                "spreadsheet_execute",
            ]
        );
        assert!(manifest.get("trust").is_none());
        assert!(manifest.get("official").is_none());
        assert!(manifest["opentopia"].get("trust").is_none());
        assert!(manifest["opentopia"].get("official").is_none());
    }

    #[test]
    fn manifest_registers_the_model_tool_without_owning_desktop_preview() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid plugin manifest");
        let opentopia = &manifest["opentopia"];
        let tool = SpreadsheetTool;

        assert_eq!(opentopia["apiVersion"], "1");
        assert_eq!(
            opentopia["contributes"]["nativeTools"][0]["id"],
            tool.name()
        );
        assert_eq!(
            opentopia["contributes"]["nativeTools"]
                .as_array()
                .expect("native tools")
                .iter()
                .skip(1)
                .map(|entry| entry["id"].as_str().expect("tool id"))
                .collect::<Vec<_>>(),
            vec![
                SpreadsheetInspectTool.name(),
                SpreadsheetDescribeTool.name(),
                SpreadsheetExecuteTool.name(),
            ]
        );
        assert_eq!(
            action_names(&tool.schema()),
            vec![
                "inspect_delimited",
                "inspect",
                "list_sheets",
                "read_range",
                "read_ranges",
                "read_rows",
                "read_columns",
                "find",
                "filter_rows",
                "validate",
                "fill_template",
                "transfer_rows",
                "export_delimited",
                "write",
                "write_rows",
                "write_columns",
                "copy_rows",
                "copy_columns",
                "batch",
            ]
        );
        assert!(opentopia["contributes"].get("previewers").is_none());
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
