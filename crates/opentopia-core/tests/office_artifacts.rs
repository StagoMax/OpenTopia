use lopdf::{content, dictionary, Document, Object, Stream};
use opentopia_core::{
    extract_document_text, extract_pdf_text, inspect_document, inspect_pdf, inspect_plugin,
    validate_document, validate_pdf, ArtifactRuntime, ArtifactRuntimeError, BasicPolicyEngine,
    ContextSourceKind, ContextSourceRef, Message, MessagePart, MessageRole, ModelContentPart,
    PermissionMode, SessionStore, SqliteSessionStore, ToolCall, ToolContext, ToolRegistry,
};
use rust_xlsxwriter::Workbook;
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

#[tokio::test]
async fn native_office_tools_read_real_thread_attachments_by_id() {
    let workspace =
        std::env::temp_dir().join(format!("opentopia-office-attachments-{}", Uuid::new_v4()));
    std::fs::create_dir(&workspace).expect("create workspace");
    let fixtures = [
        (
            "uploaded.pdf",
            "application/pdf",
            sample_pdf("Attached PDF"),
        ),
        (
            "uploaded.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            sample_docx(),
        ),
        (
            "uploaded.xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            sample_xlsx(),
        ),
    ];
    let store: Arc<dyn SessionStore> =
        Arc::new(SqliteSessionStore::open(":memory:").expect("open memory store"));
    let thread = store
        .create_thread(Some("office attachments".to_string()), workspace.clone())
        .expect("create thread");
    let mut attachment_ids = Vec::new();
    let mut message = Message::text(thread.id, MessageRole::User, "inspect these files");
    for (name, content_type, bytes) in fixtures {
        let id = Uuid::new_v4();
        let path = workspace.join(name);
        std::fs::write(&path, &bytes).expect("write attachment fixture");
        message.parts.push(MessagePart::SourceRef {
            source: ContextSourceRef {
                id,
                path,
                name: name.to_string(),
                kind: ContextSourceKind::Document,
                content_type: content_type.to_string(),
                bytes: bytes.len() as u64,
                truncated: false,
            },
        });
        attachment_ids.push(id);
    }
    store.append_message(message).expect("persist attachments");

    let policy = Arc::new(BasicPolicyEngine::new(
        workspace.clone(),
        PermissionMode::ReadOnly,
    ));
    let mut context = ToolContext::local(workspace.clone(), policy);
    context.store = Some(store.clone());
    context.thread_id = Some(thread.id);
    context.artifact_runtime =
        Arc::new(ArtifactRuntime::default().with_artifact_output_root(workspace.join("artifacts")));
    let registry = ToolRegistry::with_builtins();

    for (tool, attachment_id) in [
        ("pdf", attachment_ids[0]),
        ("document", attachment_ids[1]),
        ("spreadsheet", attachment_ids[2]),
    ] {
        let result = registry
            .get(tool)
            .expect("Office tool")
            .execute(
                ToolCall::new(
                    tool,
                    json!({ "action": "inspect", "attachmentId": attachment_id }),
                ),
                context.clone(),
            )
            .await
            .expect("execute attachment tool");
        assert_eq!(
            result.metadata["success"], true,
            "{tool}: {}",
            result.output
        );
        assert_eq!(result.metadata["provenance"], "user_attachment");
        assert_eq!(result.metadata["attachmentId"], attachment_id.to_string());
    }

    let rendered = registry
        .get("pdf")
        .expect("PDF tool")
        .execute(
            ToolCall::new(
                "pdf",
                json!({
                    "action": "render",
                    "attachmentId": attachment_ids[0],
                    "pages": [1],
                    "dpi": 96
                }),
            ),
            context.clone(),
        )
        .await
        .expect("render attachment");
    assert_eq!(rendered.metadata["success"], true);
    assert!(rendered.metadata["artifactId"].is_string());
    let artifacts = store.list_artifacts(thread.id).expect("list artifacts");
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].content_type, "image/png");
    assert_eq!(artifacts[0].metadata["page"], 1);

    let ambiguous = registry
        .get("pdf")
        .expect("PDF tool")
        .execute(
            ToolCall::new(
                "pdf",
                json!({
                    "action": "inspect",
                    "path": "uploaded.pdf",
                    "attachmentId": attachment_ids[0]
                }),
            ),
            context,
        )
        .await
        .expect_err("ambiguous source must be rejected");
    assert!(ambiguous.to_string().contains("exactly one"));
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

fn sample_xlsx() -> Vec<u8> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet
        .write_string(0, 0, "Attached XLSX")
        .expect("write spreadsheet fixture");
    workbook.save_to_buffer().expect("save spreadsheet fixture")
}
