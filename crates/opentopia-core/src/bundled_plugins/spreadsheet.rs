use super::{BundledPluginFile, BundledPluginMetadata, BundledPluginPackage, BundledPluginTrust};

const MANIFEST: &[u8] =
    include_bytes!("../../bundled-plugins/spreadsheet/.codex-plugin/plugin.json");
const CONFIGURATION_SCHEMA: &[u8] =
    include_bytes!("../../bundled-plugins/spreadsheet/configuration.schema.json");
const SPREADSHEET_SKILL: &[u8] =
    include_bytes!("../../bundled-plugins/spreadsheet/skills/manage-spreadsheets/SKILL.md");
const SPREADSHEET_SKILL_INTERFACE: &[u8] = include_bytes!(
    "../../bundled-plugins/spreadsheet/skills/manage-spreadsheets/agents/openai.yaml"
);

const FILES: &[BundledPluginFile] = &[
    BundledPluginFile {
        relative_path: ".codex-plugin/plugin.json",
        contents: MANIFEST,
    },
    BundledPluginFile {
        relative_path: "configuration.schema.json",
        contents: CONFIGURATION_SCHEMA,
    },
    BundledPluginFile {
        relative_path: "skills/manage-spreadsheets/SKILL.md",
        contents: SPREADSHEET_SKILL,
    },
    BundledPluginFile {
        relative_path: "skills/manage-spreadsheets/agents/openai.yaml",
        contents: SPREADSHEET_SKILL_INTERFACE,
    },
];

pub(super) const PACKAGE: BundledPluginPackage = BundledPluginPackage {
    metadata: BundledPluginMetadata {
        name: "spreadsheet",
        version: "6.1.0",
        trust: BundledPluginTrust::Official,
        default_enabled: true,
        native_capabilities: &[
            "spreadsheet_inspect",
            "spreadsheet_read_ranges",
            "spreadsheet_find",
            "spreadsheet_filter_rows",
            "spreadsheet_validate",
            "spreadsheet_write_range",
            "spreadsheet_copy_ranges",
            "spreadsheet_copy_rows",
            "spreadsheet_fill_ranges",
            "spreadsheet_convert_ranges",
            "spreadsheet_export_delimited",
            "spreadsheet_copy_sheet",
            "spreadsheet_delete_rows",
            "spreadsheet_delete_sheet",
        ],
    },
    files: FILES,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{SpreadsheetInspectTool, Tool};
    use serde_json::Value;

    #[test]
    fn package_and_manifest_keep_host_owned_trust_out_of_plugin_data() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid plugin manifest");

        assert_eq!(PACKAGE.metadata.name, manifest["name"]);
        assert_eq!(PACKAGE.metadata.version, manifest["version"]);
        assert_eq!(PACKAGE.metadata.trust, BundledPluginTrust::Official);
        assert!(PACKAGE.metadata.default_enabled);
        assert_eq!(manifest["skills"], "./skills/");
        assert_eq!(
            PACKAGE.metadata.native_capabilities,
            &[
                "spreadsheet_inspect",
                "spreadsheet_read_ranges",
                "spreadsheet_find",
                "spreadsheet_filter_rows",
                "spreadsheet_validate",
                "spreadsheet_write_range",
                "spreadsheet_copy_ranges",
                "spreadsheet_copy_rows",
                "spreadsheet_fill_ranges",
                "spreadsheet_convert_ranges",
                "spreadsheet_export_delimited",
                "spreadsheet_copy_sheet",
                "spreadsheet_delete_rows",
                "spreadsheet_delete_sheet",
            ]
        );
        assert!(manifest.get("trust").is_none());
        assert!(manifest.get("official").is_none());
        assert!(manifest["opentopia"].get("trust").is_none());
        assert!(manifest["opentopia"].get("official").is_none());
        assert!(manifest["interface"].get("defaultPrompt").is_none());

        let skill = std::str::from_utf8(SPREADSHEET_SKILL).expect("UTF-8 spreadsheet Skill");
        for tool_name in PACKAGE.metadata.native_capabilities {
            assert!(
                skill.contains(tool_name),
                "spreadsheet Skill must route the native tool {tool_name}"
            );
        }
    }

    #[test]
    fn manifest_registers_independent_model_tools_without_owning_desktop_preview() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid plugin manifest");
        let opentopia = &manifest["opentopia"];
        assert_eq!(opentopia["apiVersion"], "1");
        assert_eq!(
            opentopia["contributes"]["nativeTools"][0]["id"],
            SpreadsheetInspectTool.name()
        );
        assert_eq!(
            opentopia["contributes"]["nativeTools"]
                .as_array()
                .expect("native tools")
                .iter()
                .skip(1)
                .map(|entry| entry["id"].as_str().expect("tool id"))
                .collect::<Vec<_>>(),
            PACKAGE.metadata.native_capabilities[1..].to_vec()
        );
        assert!(SpreadsheetInspectTool.schema().get("oneOf").is_none());
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
