use super::{BundledPluginFile, BundledPluginMetadata, BundledPluginPackage, BundledPluginTrust};

const MANIFEST: &[u8] = include_bytes!("../../bundled-plugins/documents/.codex-plugin/plugin.json");

const FILES: &[BundledPluginFile] = &[BundledPluginFile {
    relative_path: ".codex-plugin/plugin.json",
    contents: MANIFEST,
}];

pub(super) const PACKAGE: BundledPluginPackage = BundledPluginPackage {
    metadata: BundledPluginMetadata {
        name: "documents",
        version: "2.0.0",
        trust: BundledPluginTrust::Official,
        default_enabled: true,
        native_capabilities: &["word_document"],
    },
    files: FILES,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{DocumentTool, Tool};
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
    fn manifest_registers_the_host_owned_document_tool() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid Documents manifest");
        let tool = DocumentTool;
        assert_eq!(PACKAGE.metadata.name, manifest["name"]);
        assert_eq!(PACKAGE.metadata.version, manifest["version"]);
        assert_eq!(PACKAGE.metadata.trust, BundledPluginTrust::Official);
        assert!(PACKAGE.metadata.default_enabled);
        assert_eq!(PACKAGE.metadata.native_capabilities, &["word_document"]);
        assert_eq!(
            manifest["opentopia"]["contributes"]["nativeTools"][0]["id"],
            tool.name()
        );
        assert_eq!(
            manifest["opentopia"]["contributes"]["nativeTools"][0]["runtime"],
            "builtin:word_document"
        );
        assert_eq!(
            manifest["opentopia"]["requires"]["hostCapabilities"],
            serde_json::json!(["workspace.files.v1", "nativeTool.word_document.v1"])
        );
        assert!(manifest["opentopia"]["contributes"]
            .get("previewers")
            .is_none());
        assert_eq!(
            manifest["opentopia"]["permissions"]["filesystem"],
            serde_json::json!(["workspace:read"])
        );
        let tool_schema = tool.schema();
        assert_eq!(
            action_names(&tool_schema),
            vec!["inspect", "extract", "validate"]
        );
        assert!(tool_schema["oneOf"].as_array().is_some_and(|branches| {
            branches
                .iter()
                .all(|branch| branch["properties"].get("pages").is_none())
        }));
        assert!(tool_schema["oneOf"].as_array().is_some_and(|branches| {
            branches
                .iter()
                .all(|branch| branch["properties"].get("dpi").is_none())
        }));
        assert!(manifest.get("trust").is_none());
        assert!(manifest.get("official").is_none());
        assert!(manifest["opentopia"].get("trust").is_none());
        assert!(manifest["opentopia"].get("official").is_none());
    }
}
