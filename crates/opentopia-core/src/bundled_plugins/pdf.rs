use super::{BundledPluginFile, BundledPluginMetadata, BundledPluginPackage, BundledPluginTrust};

const MANIFEST: &[u8] = include_bytes!("../../bundled-plugins/pdf/.codex-plugin/plugin.json");

const FILES: &[BundledPluginFile] = &[BundledPluginFile {
    relative_path: ".codex-plugin/plugin.json",
    contents: MANIFEST,
}];

pub(super) const PACKAGE: BundledPluginPackage = BundledPluginPackage {
    metadata: BundledPluginMetadata {
        name: "pdf",
        version: "1.0.0",
        trust: BundledPluginTrust::Official,
        default_enabled: true,
        native_capabilities: &["pdf"],
    },
    files: FILES,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{PdfTool, Tool};
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
    fn manifest_registers_the_host_owned_pdf_tool() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid PDF manifest");
        let tool = PdfTool;
        assert_eq!(PACKAGE.metadata.name, manifest["name"]);
        assert_eq!(PACKAGE.metadata.version, manifest["version"]);
        assert_eq!(PACKAGE.metadata.trust, BundledPluginTrust::Official);
        assert!(PACKAGE.metadata.default_enabled);
        assert_eq!(PACKAGE.metadata.native_capabilities, &["pdf"]);
        assert_eq!(
            manifest["opentopia"]["contributes"]["nativeTools"][0]["id"],
            tool.name()
        );
        assert_eq!(
            manifest["opentopia"]["contributes"]["nativeTools"][0]["runtime"],
            "builtin:pdf"
        );
        assert_eq!(
            manifest["opentopia"]["requires"]["hostCapabilities"],
            serde_json::json!([
                "workspace.files.v1",
                "artifact.runtime.v1",
                "nativeTool.pdf.v1"
            ])
        );
        assert_eq!(
            manifest["opentopia"]["permissions"]["filesystem"],
            serde_json::json!(["workspace:read"])
        );
        let tool_schema = tool.schema();
        assert_eq!(
            action_names(&tool_schema),
            vec!["inspect", "extract", "render", "validate"]
        );
        assert!(tool_schema["oneOf"].as_array().is_some_and(|branches| {
            branches
                .iter()
                .filter_map(|branch| branch["properties"].get("pages"))
                .all(|pages| pages["items"]["minimum"] == serde_json::json!(1.0))
        }));
        assert!(manifest.get("trust").is_none());
        assert!(manifest.get("official").is_none());
        assert!(manifest["opentopia"].get("trust").is_none());
        assert!(manifest["opentopia"].get("official").is_none());
    }
}
