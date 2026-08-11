use lopdf::{content, dictionary, Document, Object, Stream};
use opentopia_core::{
    extract_document_text, extract_pdf_text, inspect_document, inspect_pdf, inspect_plugin,
    validate_document, validate_pdf, ArtifactRuntime, ArtifactRuntimeError, BasicPolicyEngine,
    ModelContentPart, PermissionMode, ToolCall, ToolContext, ToolRegistry,
};
use serde_json::json;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[test]
fn bundled_office_plugins_register_independent_native_tools() {
    let plugin_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("bundled-plugins");
    for (plugin, tool) in [("pdf", "pdf"), ("documents", "document")] {
        let descriptor = inspect_plugin(&plugin_root.join(plugin)).expect("inspect bundled plugin");
        assert!(descriptor.is_compatible(), "{:?}", descriptor.issues);
        assert_eq!(descriptor.capability_manifest.contributions.len(), 1);
        assert!(descriptor
            .capability_manifest
            .contributions
            .iter()
            .any(|contribution| contribution.local_id == tool));
    }
    let registry = ToolRegistry::with_builtins();
    assert!(registry.get("pdf").is_some());
    assert!(registry.get("document").is_some());
}

#[tokio::test]
async fn pdf_pipeline_inspects_extracts_validates_and_renders_png() {
    let pdf = sample_pdf("OpenTopia PDF");
    let path = Path::new("sample.pdf");
    assert_eq!(inspect_pdf(path, &pdf).expect("inspect PDF").page_count, 1);
    assert!(extract_pdf_text(path, &pdf, &[1], 10_000)
        .expect("extract PDF")
        .pages[0]
        .text
        .contains("OpenTopia PDF"));
    assert!(validate_pdf(path, &pdf).report.valid);

    let rendered = ArtifactRuntime::default()
        .render_pdf(pdf, vec![1], 96)
        .await
        .expect("render PDF");
    assert_eq!(rendered.len(), 1);
    assert!(rendered[0].png.starts_with(b"\x89PNG\r\n\x1a\n"));

    let error = ArtifactRuntime::default()
        .render_pdf(sample_pdf("invalid page"), vec![0], 96)
        .await
        .expect_err("page numbers are one-based");
    assert!(matches!(
        error,
        ArtifactRuntimeError::PageOutOfRange { page: 0, .. }
    ));

    let oversized_page = ArtifactRuntime::default()
        .render_pdf(
            sample_pdf_with_media_box("wide page", 100_000, 792),
            vec![1],
            96,
        )
        .await
        .expect("render oversized media box within the pixel budget");
    assert!(oversized_page[0].width <= 2_400);
    assert!(oversized_page[0].height <= 2_400);
}

#[tokio::test]
async fn native_tools_read_through_the_workspace_environment_and_return_typed_png() {
    let workspace = std::env::temp_dir().join(format!("opentopia-office-{}", Uuid::new_v4()));
    std::fs::create_dir(&workspace).expect("create workspace");
    std::fs::write(workspace.join("sample.pdf"), sample_pdf("Native PDF"))
        .expect("write PDF fixture");
    std::fs::write(workspace.join("sample.docx"), sample_docx()).expect("write DOCX fixture");
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolContext::local(workspace.clone(), policy.clone());
    let second_context = ToolContext::local(workspace.clone(), policy);
    assert!(Arc::ptr_eq(
        &context.artifact_runtime,
        &second_context.artifact_runtime
    ));
    let registry = ToolRegistry::with_builtins();

    let pdf = registry
        .get("pdf")
        .expect("PDF tool")
        .execute(
            ToolCall::new(
                "pdf",
                json!({ "action": "render", "path": "sample.pdf", "pages": [1], "dpi": 96 }),
            ),
            context.clone(),
        )
        .await
        .expect("execute PDF tool");
    assert_eq!(pdf.metadata["success"], true);
    assert!(pdf
        .content
        .iter()
        .any(|part| matches!(part, ModelContentPart::Image { content_type, data } if content_type == "image/png" && data.starts_with(b"\x89PNG"))));

    let document = registry
        .get("document")
        .expect("Document tool")
        .execute(
            ToolCall::new(
                "document",
                json!({ "action": "inspect", "path": "sample.docx" }),
            ),
            context,
        )
        .await
        .expect("execute Document tool");
    assert_eq!(document.metadata["success"], true);
    assert!(document.output.contains("paragraphCount"));
    std::fs::remove_dir_all(workspace).expect("remove workspace");
}

#[test]
fn docx_pipeline_inspects_extracts_and_reports_preservation_risks() {
    let docx = sample_docx();
    let path = Path::new("sample.docx");
    let inspection = inspect_document(path, &docx).expect("inspect DOCX");
    assert_eq!(inspection.paragraph_count, 2);
    assert_eq!(inspection.table_count, 1);
    assert_eq!(inspection.header_count, 1);

    let extraction = extract_document_text(path, &docx, true, 10_000).expect("extract DOCX");
    assert!(extraction.parts[0].text.contains("OpenTopia DOCX"));
    assert!(extraction
        .parts
        .iter()
        .any(|part| part.text.contains("Header")));
    assert!(validate_document(path, &docx).report.valid);

    let invalid = sample_docx_with_main("<garbage/>");
    assert!(!validate_document(path, &invalid).report.valid);
}

fn sample_pdf(text: &str) -> Vec<u8> {
    sample_pdf_with_media_box(text, 612, 792)
}

fn sample_pdf_with_media_box(text: &str, width: i64, height: i64) -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let page_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = document.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let content = content::Content {
        operations: vec![
            content::Operation::new("BT", vec![]),
            content::Operation::new("Tf", vec!["F1".into(), 12.into()]),
            content::Operation::new("Td", vec![48.into(), 760.into()]),
            content::Operation::new("Tj", vec![Object::string_literal(text)]),
            content::Operation::new("ET", vec![]),
        ],
    };
    let content_id = document.add_object(Stream::new(
        dictionary! {},
        content.encode().expect("encode PDF"),
    ));
    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), width.into(), height.into()],
        }),
    );
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).expect("save PDF");
    bytes
}

fn sample_docx() -> Vec<u8> {
    sample_docx_with_main(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>OpenTopia DOCX</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#,
    )
}

fn sample_docx_with_main(main_document: &str) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default();
    let files = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        ("word/document.xml", main_document),
        (
            "word/header1.xml",
            r#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>Header</w:t></w:r></w:p></w:hdr>"#,
        ),
    ];
    for (name, contents) in files {
        writer.start_file(name, options).expect("start DOCX part");
        writer
            .write_all(contents.as_bytes())
            .expect("write DOCX part");
    }
    writer.finish().expect("finish DOCX").into_inner()
}
