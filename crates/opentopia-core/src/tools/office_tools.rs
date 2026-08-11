use super::*;
use crate::artifact_runtime::{ArtifactRuntimeError, RenderedPage, MAX_ARTIFACT_INPUT_BYTES};
use crate::document::{
    extract_document_text, inspect_document, validate_document, DocumentError,
    MAX_DOCUMENT_EXTRACT_CHARACTERS,
};
use crate::pdf::{
    extract_pdf_text, inspect_pdf, validate_pdf, PdfError, MAX_PDF_EXTRACT_CHARACTERS,
};

const DEFAULT_RENDER_DPI: u16 = 144;

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PdfToolAction {
    Inspect,
    Extract,
    Render,
    Validate,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PdfToolInput {
    action: PdfToolAction,
    /// Workspace-relative PDF path.
    path: String,
    /// One-based page numbers. Extract defaults to all pages; render defaults to page 1.
    #[serde(default)]
    #[schemars(length(max = 8), inner(range(min = 1)))]
    pages: Vec<u32>,
    /// Maximum extracted characters.
    #[serde(default)]
    #[schemars(range(min = 1, max = 200000))]
    max_characters: Option<usize>,
    /// Render resolution in dots per inch.
    #[serde(default)]
    #[schemars(range(min = 36, max = 288))]
    dpi: Option<u16>,
}

pub struct PdfTool;

#[async_trait]
impl TypedTool for PdfTool {
    type Input = PdfToolInput;

    fn name(&self) -> &str {
        "pdf"
    }

    fn description(&self) -> &str {
        "Inspect PDF structure, extract bounded page text, render selected pages as PNG images, or run deterministic validation. Page numbers are one-based. Rendering and extraction are complementary; neither replaces the other."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let file = tool_resource_key("file", &input.path);
        if matches!(input.action, PdfToolAction::Render) {
            ToolExecutionPolicy::read_only(vec!["artifact-runtime:pdf".to_string(), file])
        } else {
            ToolExecutionPolicy::read_only(vec![file])
        }
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        ToolExecutionIntent::observation([PathBuf::from(&input.path)])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let (path, bytes) = read_artifact_input(&ctx, &input.path).await?;
        match input.action {
            PdfToolAction::Inspect => {
                let worker_path = path.clone();
                let result = tokio::task::spawn_blocking(move || inspect_pdf(&worker_path, &bytes))
                    .await
                    .context("PDF inspect worker failed")?;
                match result {
                    Ok(result) => structured_success(call_id, "pdf", "inspect", result),
                    Err(error) => Ok(pdf_error_result(call_id, "inspect", error)),
                }
            }
            PdfToolAction::Extract => {
                let worker_path = path.clone();
                let pages = input.pages;
                let max_characters = input.max_characters.unwrap_or(MAX_PDF_EXTRACT_CHARACTERS);
                let result = tokio::task::spawn_blocking(move || {
                    extract_pdf_text(&worker_path, &bytes, &pages, max_characters)
                })
                .await
                .context("PDF extract worker failed")?;
                match result {
                    Ok(result) => structured_success(call_id, "pdf", "extract", result),
                    Err(error) => Ok(pdf_error_result(call_id, "extract", error)),
                }
            }
            PdfToolAction::Render => {
                let dpi = checked_dpi(input.dpi, call_id, "pdf")?;
                let pages = normalized_render_pages(input.pages);
                let worker_path = path.clone();
                let validation_bytes = bytes.clone();
                let parsed = tokio::task::spawn_blocking(move || {
                    inspect_pdf(&worker_path, &validation_bytes)
                })
                .await
                .context("PDF render validation worker failed")?;
                if let Err(error) = parsed {
                    return Ok(pdf_error_result(call_id, "render", error));
                }
                match ctx
                    .artifact_runtime
                    .render_pdf_with_cancel(bytes, pages, dpi, ctx.cancel.clone())
                    .await
                {
                    Ok(pages) => render_success(call_id, "pdf", &path, dpi, pages),
                    Err(error) => Ok(runtime_error_result(call_id, "pdf", "render", error)),
                }
            }
            PdfToolAction::Validate => {
                let worker_path = path.clone();
                let result =
                    tokio::task::spawn_blocking(move || validate_pdf(&worker_path, &bytes))
                        .await
                        .context("PDF validate worker failed")?;
                structured_success(call_id, "pdf", "validate", result)
            }
        }
    }
}

impl_typed_tool!(PdfTool);

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum DocumentToolAction {
    Inspect,
    Extract,
    Render,
    Validate,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentToolInput {
    action: DocumentToolAction,
    /// Workspace-relative DOCX path.
    path: String,
    /// Include headers, footers, comments, footnotes, and endnotes during extraction.
    #[serde(default)]
    include_related_parts: bool,
    /// Maximum extracted characters.
    #[serde(default)]
    #[schemars(range(min = 1, max = 200000))]
    max_characters: Option<usize>,
    /// One-based pages to render after LibreOffice conversion; defaults to page 1.
    #[serde(default)]
    #[schemars(length(max = 8), inner(range(min = 1)))]
    pages: Vec<u32>,
    /// Render resolution in dots per inch.
    #[serde(default)]
    #[schemars(range(min = 36, max = 288))]
    dpi: Option<u16>,
}

pub struct DocumentTool;

#[async_trait]
impl TypedTool for DocumentTool {
    type Input = DocumentToolInput;

    fn name(&self) -> &str {
        "document"
    }

    fn description(&self) -> &str {
        "Inspect DOCX package structure, extract bounded WordprocessingML text, render selected pages through LibreOffice as typed PNG images, or validate package integrity and preservation risks. This tool never rewrites the source file."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let file = tool_resource_key("file", &input.path);
        match input.action {
            DocumentToolAction::Render => ToolExecutionPolicy {
                read_only: false,
                idempotent: true,
                parallel_safe: true,
                side_effect: ToolSideEffect::Process,
                resource_keys: vec!["artifact-runtime:libreoffice".to_string(), file],
            },
            _ => ToolExecutionPolicy::read_only(vec![file]),
        }
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        let intent = ToolExecutionIntent::observation([PathBuf::from(&input.path)]);
        if matches!(input.action, DocumentToolAction::Render) {
            intent.with_process_lifetime(ProcessLifetime::OneShot)
        } else {
            intent
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let (path, bytes) = read_artifact_input(&ctx, &input.path).await?;
        match input.action {
            DocumentToolAction::Inspect => {
                let worker_path = path.clone();
                let result =
                    tokio::task::spawn_blocking(move || inspect_document(&worker_path, &bytes))
                        .await
                        .context("DOCX inspect worker failed")?;
                match result {
                    Ok(result) => structured_success(call_id, "document", "inspect", result),
                    Err(error) => Ok(document_error_result(call_id, "inspect", error)),
                }
            }
            DocumentToolAction::Extract => {
                let worker_path = path.clone();
                let include_related_parts = input.include_related_parts;
                let max_characters = input
                    .max_characters
                    .unwrap_or(MAX_DOCUMENT_EXTRACT_CHARACTERS);
                let result = tokio::task::spawn_blocking(move || {
                    extract_document_text(
                        &worker_path,
                        &bytes,
                        include_related_parts,
                        max_characters,
                    )
                })
                .await
                .context("DOCX extract worker failed")?;
                match result {
                    Ok(result) => structured_success(call_id, "document", "extract", result),
                    Err(error) => Ok(document_error_result(call_id, "extract", error)),
                }
            }
            DocumentToolAction::Render => {
                let dpi = checked_dpi(input.dpi, call_id, "document")?;
                let pages = normalized_render_pages(input.pages);
                let worker_path = path.clone();
                let validation_bytes = bytes.clone();
                let parsed = tokio::task::spawn_blocking(move || {
                    inspect_document(&worker_path, &validation_bytes)
                })
                .await
                .context("DOCX render validation worker failed")?;
                if let Err(error) = parsed {
                    return Ok(document_error_result(call_id, "render", error));
                }
                match ctx
                    .artifact_runtime
                    .render_docx(
                        bytes,
                        pages,
                        dpi,
                        ctx.cancel.clone(),
                        ctx.sandbox_config.clone(),
                    )
                    .await
                {
                    Ok(pages) => render_success(call_id, "document", &path, dpi, pages),
                    Err(error) => Ok(runtime_error_result(call_id, "document", "render", error)),
                }
            }
            DocumentToolAction::Validate => {
                let worker_path = path.clone();
                let result =
                    tokio::task::spawn_blocking(move || validate_document(&worker_path, &bytes))
                        .await
                        .context("DOCX validate worker failed")?;
                let mut value = serde_json::to_value(result)?;
                if let Some(object) = value.as_object_mut() {
                    object.insert(
                        "libreOfficeAvailable".to_string(),
                        json!(ctx.artifact_runtime.libreoffice_available()),
                    );
                }
                structured_value_success(call_id, "document", "validate", value)
            }
        }
    }
}

impl_typed_tool!(DocumentTool);

async fn read_artifact_input(
    ctx: &ToolContext,
    requested: &str,
) -> anyhow::Result<(PathBuf, Vec<u8>)> {
    let requested = requested.trim();
    anyhow::ensure!(!requested.is_empty(), "artifact tool requires path");
    let logical_path = normalize_workspace_path(&ctx.workspace_root, requested)?;
    enforce_read_policy(ctx, &logical_path)?;
    let resolved_path = ctx.environment.resolve_read_path(&logical_path)?;
    let read = ctx
        .environment
        .read_file(FileReadRequest::new(&resolved_path).with_max_bytes(MAX_ARTIFACT_INPUT_BYTES))
        .await?;
    Ok((read.path, read.bytes))
}

fn normalized_render_pages(pages: Vec<u32>) -> Vec<u32> {
    let pages = if pages.is_empty() { vec![1] } else { pages };
    let mut seen = HashSet::new();
    pages
        .into_iter()
        .filter(|page| seen.insert(*page))
        .collect()
}

fn checked_dpi(value: Option<u16>, call_id: Uuid, tool_name: &str) -> anyhow::Result<u16> {
    let dpi = value.unwrap_or(DEFAULT_RENDER_DPI);
    if !(36..=288).contains(&dpi) {
        anyhow::bail!("{tool_name} render dpi must be between 36 and 288 (call {call_id})");
    }
    Ok(dpi)
}

fn structured_success<T: Serialize>(
    call_id: Uuid,
    tool_name: &str,
    action: &str,
    result: T,
) -> anyhow::Result<ToolResult> {
    structured_value_success(call_id, tool_name, action, serde_json::to_value(result)?)
}

fn structured_value_success(
    call_id: Uuid,
    tool_name: &str,
    action: &str,
    value: Value,
) -> anyhow::Result<ToolResult> {
    Ok(ToolResult {
        call_id,
        output: serde_json::to_string_pretty(&value)?,
        content: vec![ModelContentPart::json(value)],
        metadata: json!({
            "toolName": tool_name,
            "action": action,
            "success": true
        }),
    })
}

fn render_success(
    call_id: Uuid,
    tool_name: &str,
    path: &Path,
    dpi: u16,
    pages: Vec<RenderedPage>,
) -> anyhow::Result<ToolResult> {
    let summaries = pages
        .iter()
        .map(|page| {
            json!({
                "page": page.page,
                "width": page.width,
                "height": page.height,
                "pngBytes": page.png.len()
            })
        })
        .collect::<Vec<_>>();
    let value = json!({
        "path": path,
        "dpi": dpi,
        "pages": summaries
    });
    let mut content = vec![ModelContentPart::json(value.clone())];
    content.extend(
        pages
            .into_iter()
            .map(|page| ModelContentPart::image("image/png", page.png)),
    );
    Ok(ToolResult {
        call_id,
        output: serde_json::to_string_pretty(&value)?,
        content,
        metadata: json!({
            "toolName": tool_name,
            "action": "render",
            "success": true,
            "renderedPages": summaries.len()
        }),
    })
}

fn pdf_error_result(call_id: Uuid, action: &str, error: PdfError) -> ToolResult {
    domain_error_result(call_id, "pdf", action, "pdf_error", error.to_string())
}

fn document_error_result(call_id: Uuid, action: &str, error: DocumentError) -> ToolResult {
    domain_error_result(
        call_id,
        "document",
        action,
        "document_error",
        error.to_string(),
    )
}

fn runtime_error_result(
    call_id: Uuid,
    tool_name: &str,
    action: &str,
    error: ArtifactRuntimeError,
) -> ToolResult {
    let code = match &error {
        ArtifactRuntimeError::LibreOfficeUnavailable => "dependency_unavailable",
        ArtifactRuntimeError::Cancelled => "cancelled",
        ArtifactRuntimeError::PdfRenderTimeout { .. } => "render_timeout",
        _ => "render_error",
    };
    domain_error_result(call_id, tool_name, action, code, error.to_string())
}

fn domain_error_result(
    call_id: Uuid,
    tool_name: &str,
    action: &str,
    code: &str,
    message: String,
) -> ToolResult {
    let value = json!({ "code": code, "message": message });
    ToolResult {
        call_id,
        output: serde_json::to_string_pretty(&value).unwrap_or_else(|_| message.clone()),
        content: vec![ModelContentPart::json(value)],
        metadata: json!({
            "toolName": tool_name,
            "action": action,
            "success": false,
            "errorCode": code,
            "error": message
        }),
    }
}
