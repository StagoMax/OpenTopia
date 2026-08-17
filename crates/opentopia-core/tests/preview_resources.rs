use opentopia_core::{
    resolve_local_preview, write_preview_content, PreviewError, PreviewSource,
    MAX_PREVIEW_CONTENT_BYTES,
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("opentopia-resource-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn local_resource_writes_are_capability_scoped_and_revision_checked() {
    let directory = TestDirectory::new();
    let path = directory.path().join("说明.md");
    std::fs::write(&path, b"# Before\n").expect("write fixture");
    let resource_id = Uuid::new_v4();
    let preview = resolve_local_preview(resource_id, &path).expect("resolve local resource");

    assert_eq!(preview.descriptor.source, PreviewSource::Local);
    assert!(preview.descriptor.id.contains(&resource_id.to_string()));
    assert!(!preview.descriptor.id.contains("说明"));
    assert!(preview.descriptor.capabilities.read);
    assert!(preview.descriptor.capabilities.write);
    assert!(preview.descriptor.capabilities.watch);

    write_preview_content(
        &preview,
        &preview.descriptor.revision,
        b"# After\n",
        MAX_PREVIEW_CONTENT_BYTES,
    )
    .expect("save resource");
    assert_eq!(std::fs::read(&path).unwrap(), b"# After\n");

    let current = resolve_local_preview(resource_id, &path).expect("refresh resource");
    let error = write_preview_content(
        &current,
        &preview.descriptor.revision,
        b"stale",
        MAX_PREVIEW_CONTENT_BYTES,
    )
    .expect_err("reject a stale revision");
    assert!(matches!(error, PreviewError::RevisionConflict { .. }));
}

#[test]
fn local_resource_save_preserves_an_existing_utf8_bom() {
    let directory = TestDirectory::new();
    let path = directory.path().join("bom.md");
    std::fs::write(&path, b"\xef\xbb\xbf# Before\r\n").expect("write BOM fixture");
    let preview = resolve_local_preview(Uuid::new_v4(), &path).expect("resolve local resource");

    write_preview_content(
        &preview,
        &preview.descriptor.revision,
        b"# After\r\n",
        MAX_PREVIEW_CONTENT_BYTES,
    )
    .expect("save BOM resource");
    assert_eq!(std::fs::read(path).unwrap(), b"\xef\xbb\xbf# After\r\n");
}
