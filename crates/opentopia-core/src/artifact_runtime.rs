use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, RenderCache, RenderSettings};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

pub const MAX_ARTIFACT_INPUT_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_RENDERED_PAGES: usize = 8;
pub const MAX_RENDERED_PAGE_EDGE: u16 = 2_400;
pub const MAX_RENDERED_TOTAL_PIXELS: u64 = 24_000_000;
pub const MAX_RENDERED_TOTAL_BYTES: usize = 8 * 1024 * 1024;
const PDF_RENDER_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    pub code: String,
    pub severity: ValidationSeverity,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub valid: bool,
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn from_issues(issues: Vec<ValidationIssue>) -> Self {
        let valid = !issues
            .iter()
            .any(|issue| issue.severity == ValidationSeverity::Error);
        Self { valid, issues }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RenderedPage {
    pub page: u32,
    pub width: u16,
    pub height: u16,
    #[serde(skip)]
    pub png: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactRuntimeError {
    #[error("PDF rendering failed: {0}")]
    PdfRender(String),
    #[error("requested page {page} is outside the PDF page range 1..={page_count}")]
    PageOutOfRange { page: u32, page_count: usize },
    #[error("render requests are limited to {limit} pages; received {actual}")]
    TooManyPages { actual: usize, limit: usize },
    #[error("rendered output exceeds the {limit} byte limit")]
    RenderOutputTooLarge { limit: usize },
    #[error("artifact rendering was cancelled")]
    Cancelled,
    #[error("PDF rendering exceeded the {seconds} second timeout")]
    PdfRenderTimeout { seconds: u64 },
}

pub trait PdfBackend: Send + Sync {
    fn render(
        &self,
        pdf_bytes: Vec<u8>,
        pages: &[u32],
        dpi: u16,
    ) -> Result<Vec<RenderedPage>, ArtifactRuntimeError>;
}

#[derive(Debug, Default)]
pub struct HayroPdfBackend;

impl PdfBackend for HayroPdfBackend {
    fn render(
        &self,
        pdf_bytes: Vec<u8>,
        pages: &[u32],
        dpi: u16,
    ) -> Result<Vec<RenderedPage>, ArtifactRuntimeError> {
        if pages.len() > MAX_RENDERED_PAGES {
            return Err(ArtifactRuntimeError::TooManyPages {
                actual: pages.len(),
                limit: MAX_RENDERED_PAGES,
            });
        }
        let pdf = Pdf::new(pdf_bytes)
            .map_err(|error| ArtifactRuntimeError::PdfRender(format!("{error:?}")))?;
        let page_count = pdf.pages().len();
        let requested = if pages.is_empty() {
            vec![1]
        } else {
            pages.to_vec()
        };
        let mut total_pixels = 0_u64;
        let mut total_bytes = 0_usize;
        let mut rendered_pages = Vec::with_capacity(requested.len());
        let cache = RenderCache::new();
        let interpreter_settings = InterpreterSettings::default();
        let requested_scale = f32::from(dpi.clamp(36, 288)) / 72.0;

        for page_number in requested {
            if page_number == 0 {
                return Err(ArtifactRuntimeError::PageOutOfRange {
                    page: page_number,
                    page_count,
                });
            }
            let index = usize::try_from(page_number.saturating_sub(1)).unwrap_or(usize::MAX);
            let page = pdf
                .pages()
                .get(index)
                .ok_or(ArtifactRuntimeError::PageOutOfRange {
                    page: page_number,
                    page_count,
                })?;
            let (page_width, page_height) = page.render_dimensions();
            if !page_width.is_finite()
                || !page_height.is_finite()
                || page_width <= 0.0
                || page_height <= 0.0
            {
                return Err(ArtifactRuntimeError::PdfRender(format!(
                    "page {page_number} has invalid dimensions"
                )));
            }
            let max_edge = f32::from(MAX_RENDERED_PAGE_EDGE);
            let scale = requested_scale
                .min(max_edge / page_width)
                .min(max_edge / page_height);
            if !scale.is_finite() || scale <= 0.0 {
                return Err(ArtifactRuntimeError::PdfRender(format!(
                    "page {page_number} cannot be scaled within the render limits"
                )));
            }
            let width = (page_width * scale)
                .floor()
                .clamp(1.0, f32::from(MAX_RENDERED_PAGE_EDGE)) as u16;
            let height = (page_height * scale)
                .floor()
                .clamp(1.0, f32::from(MAX_RENDERED_PAGE_EDGE)) as u16;
            total_pixels = total_pixels.saturating_add(u64::from(width) * u64::from(height));
            if total_pixels > MAX_RENDERED_TOTAL_PIXELS {
                return Err(ArtifactRuntimeError::PdfRender(format!(
                    "rendered pages exceed the {MAX_RENDERED_TOTAL_PIXELS} pixel limit"
                )));
            }
            let pixmap = render(
                page,
                &cache,
                &interpreter_settings,
                &RenderSettings {
                    x_scale: scale,
                    y_scale: scale,
                    width: Some(width),
                    height: Some(height),
                    bg_color: WHITE,
                    ..Default::default()
                },
            );
            let width = pixmap.width();
            let height = pixmap.height();
            let png = pixmap
                .into_png()
                .map_err(|error| ArtifactRuntimeError::PdfRender(error.to_string()))?;
            total_bytes = total_bytes.saturating_add(png.len());
            if total_bytes > MAX_RENDERED_TOTAL_BYTES {
                return Err(ArtifactRuntimeError::RenderOutputTooLarge {
                    limit: MAX_RENDERED_TOTAL_BYTES,
                });
            }
            rendered_pages.push(RenderedPage {
                page: page_number,
                width,
                height,
                png,
            });
        }
        Ok(rendered_pages)
    }
}

#[derive(Clone)]
pub struct ArtifactRuntime {
    pdf_backend: Arc<dyn PdfBackend>,
    artifact_output_root: Option<PathBuf>,
    pdf_renders: Arc<Semaphore>,
}

impl Default for ArtifactRuntime {
    fn default() -> Self {
        Self {
            pdf_backend: Arc::new(HayroPdfBackend),
            artifact_output_root: discover_artifact_output_root(),
            pdf_renders: Arc::new(Semaphore::new(1)),
        }
    }
}

impl ArtifactRuntime {
    pub fn shared() -> Arc<Self> {
        static RUNTIME: OnceLock<Arc<ArtifactRuntime>> = OnceLock::new();
        RUNTIME
            .get_or_init(|| Arc::new(ArtifactRuntime::default()))
            .clone()
    }

    pub fn with_pdf_backend(pdf_backend: Arc<dyn PdfBackend>) -> Self {
        Self {
            pdf_backend,
            ..Self::default()
        }
    }

    pub fn with_artifact_output_root(mut self, artifact_output_root: PathBuf) -> Self {
        self.artifact_output_root = Some(artifact_output_root);
        self
    }

    pub fn artifact_output_root(&self) -> Option<&Path> {
        self.artifact_output_root.as_deref()
    }

    pub async fn render_pdf(
        &self,
        pdf_bytes: Vec<u8>,
        pages: Vec<u32>,
        dpi: u16,
    ) -> Result<Vec<RenderedPage>, ArtifactRuntimeError> {
        self.render_pdf_with_cancel(pdf_bytes, pages, dpi, None)
            .await
    }

    pub async fn render_pdf_with_cancel(
        &self,
        pdf_bytes: Vec<u8>,
        pages: Vec<u32>,
        dpi: u16,
        cancel: Option<CancellationToken>,
    ) -> Result<Vec<RenderedPage>, ArtifactRuntimeError> {
        let cancel = cancel.unwrap_or_default();
        let acquire =
            tokio::time::timeout(PDF_RENDER_TIMEOUT, self.pdf_renders.clone().acquire_owned());
        let permit = tokio::select! {
            permit = acquire => permit
                .map_err(|_| ArtifactRuntimeError::PdfRenderTimeout {
                    seconds: PDF_RENDER_TIMEOUT.as_secs(),
                })?
                .map_err(|error| ArtifactRuntimeError::PdfRender(error.to_string()))?,
            _ = cancel.cancelled() => return Err(ArtifactRuntimeError::Cancelled),
        };
        let backend = self.pdf_backend.clone();
        let mut worker = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            backend.render(pdf_bytes, &pages, dpi)
        });
        tokio::select! {
            result = &mut worker => result
                .map_err(|error| ArtifactRuntimeError::PdfRender(error.to_string()))?,
            _ = cancel.cancelled() => {
                worker.abort();
                Err(ArtifactRuntimeError::Cancelled)
            },
            _ = tokio::time::sleep(PDF_RENDER_TIMEOUT) => {
                worker.abort();
                Err(ArtifactRuntimeError::PdfRenderTimeout {
                    seconds: PDF_RENDER_TIMEOUT.as_secs(),
                })
            },
        }
    }
}

fn discover_artifact_output_root() -> Option<PathBuf> {
    if let Some(path) = env::var_os("OPENTOPIA_ARTIFACTS_DIR") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    if cfg!(windows) {
        return env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("OpenTopia").join("artifacts"));
    }
    if cfg!(target_os = "macos") {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .map(|path| path.join("Library/Application Support/OpenTopia/artifacts"));
    }
    env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".local/share"))
        })
        .map(|path| path.join("opentopia").join("artifacts"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_report_fails_only_on_errors() {
        let warning = ValidationIssue {
            code: "warning".to_string(),
            severity: ValidationSeverity::Warning,
            message: "check visually".to_string(),
            location: None,
        };
        assert!(ValidationReport::from_issues(vec![warning.clone()]).valid);
        let error = ValidationIssue {
            severity: ValidationSeverity::Error,
            ..warning
        };
        assert!(!ValidationReport::from_issues(vec![error]).valid);
    }
}
