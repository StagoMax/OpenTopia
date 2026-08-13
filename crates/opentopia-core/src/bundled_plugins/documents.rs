use super::{BundledPluginFile, BundledPluginMetadata, BundledPluginPackage, BundledPluginTrust};

const MANIFEST: &[u8] = include_bytes!("../../bundled-plugins/documents/.codex-plugin/plugin.json");

const FILES: &[BundledPluginFile] = &[BundledPluginFile {
    relative_path: ".codex-plugin/plugin.json",
    contents: MANIFEST,
}];

pub(super) const PACKAGE: BundledPluginPackage = BundledPluginPackage {
    metadata: BundledPluginMetadata {
        name: "documents",
        version: "1.3.0",
        trust: BundledPluginTrust::Official,
        default_enabled: true,
        native_capabilities: &["document"],
    },
    files: FILES,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{DocumentTool, Tool};
    use serde_json::Value;

    #[test]
    fn manifest_registers_the_host_owned_document_tool() {
        let manifest: Value = serde_json::from_slice(MANIFEST).expect("valid Documents manifest");
        let tool = DocumentTool;
        assert_eq!(PACKAGE.metadata.name, manifest["name"]);
        assert_eq!(PACKAGE.metadata.version, manifest["version"]);
        assert_eq!(PACKAGE.metadata.trust, BundledPluginTrust::Official);
        assert!(PACKAGE.metadata.default_enabled);
        assert_eq!(PACKAGE.metadata.native_capabilities, &["document"]);
        assert_eq!(
            manifest["opentopia"]["contributes"]["nativeTools"][0]["id"],
            tool.name()
        );
        assert_eq!(
            manifest["opentopia"]["contributes"]["nativeTools"][0]["runtime"],
            "builtin:document"
        );
        assert_eq!(
            manifest["opentopia"]["requires"]["hostCapabilities"],
            serde_json::json!(["workspace.files.v1", "nativeTool.document.v1"])
        );
        assert!(manifest["opentopia"]["contributes"]
            .get("previewers")
            .is_none());
        assert_eq!(
            manifest["opentopia"]["permissions"]["filesystem"],
            serde_json::json!(["workspace:read"])
        );
        assert_eq!(
            tool.schema()["properties"]["action"]["enum"],
            serde_json::json!(["inspect", "extract", "validate"])
        );
        assert!(tool.schema()["properties"].get("pages").is_none());
        assert!(tool.schema()["properties"].get("dpi").is_none());
        assert!(manifest.get("trust").is_none());
        assert!(manifest.get("official").is_none());
        assert!(manifest["opentopia"].get("trust").is_none());
        assert!(manifest["opentopia"].get("official").is_none());
    }
}
