use crate::execution::{
    ExecRequest, ExecutionContext, ExecutionEnvironment, ExecutionRequirements,
    LocalExecutionEnvironment, ResourceLimit,
};
use crate::sandbox::{
    LocalSandboxConfig, NetworkPolicy, OsSandboxMode, SandboxMode, WindowsSandboxBackend,
};
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, RenderCache, RenderSettings};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

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
    #[error("artifact staging failed: {0}")]
    Staging(String),
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
    #[error("LibreOffice is not available; set OPENTOPIA_LIBREOFFICE_PATH or install LibreOffice")]
    LibreOfficeUnavailable,
    #[error("LibreOffice conversion failed: {0}")]
    LibreOffice(String),
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
    libreoffice_path: Option<PathBuf>,
    pdf_renders: Arc<Semaphore>,
    office_processes: Arc<Semaphore>,
}

impl Default for ArtifactRuntime {
    fn default() -> Self {
        Self {
            pdf_backend: Arc::new(HayroPdfBackend),
            libreoffice_path: discover_libreoffice(),
            pdf_renders: Arc::new(Semaphore::new(1)),
            office_processes: Arc::new(Semaphore::new(1)),
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

    pub fn libreoffice_available(&self) -> bool {
        self.libreoffice_path.is_some()
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

    pub async fn render_docx(
        &self,
        docx_bytes: Vec<u8>,
        pages: Vec<u32>,
        dpi: u16,
        cancel: Option<CancellationToken>,
        sandbox_config: Option<LocalSandboxConfig>,
    ) -> Result<Vec<RenderedPage>, ArtifactRuntimeError> {
        let pdf = self
            .convert_docx_to_pdf(&docx_bytes, cancel.clone(), sandbox_config)
            .await?;
        self.render_pdf_with_cancel(pdf, pages, dpi, cancel).await
    }

    async fn convert_docx_to_pdf(
        &self,
        docx_bytes: &[u8],
        cancel: Option<CancellationToken>,
        sandbox_config: Option<LocalSandboxConfig>,
    ) -> Result<Vec<u8>, ArtifactRuntimeError> {
        let executable = self
            .libreoffice_path
            .as_ref()
            .ok_or(ArtifactRuntimeError::LibreOfficeUnavailable)?;
        let cancel = cancel.unwrap_or_default();
        let _permit = tokio::select! {
            permit = self.office_processes.acquire() => permit
                .map_err(|error| ArtifactRuntimeError::LibreOffice(error.to_string()))?,
            _ = cancel.cancelled() => return Err(ArtifactRuntimeError::Cancelled),
        };
        let staging = ArtifactSession::new()?;
        let input = staging.root.join("input.docx");
        let output_dir = staging.root.join("output");
        let profile = staging.root.join("profile");
        fs::create_dir(&output_dir)
            .and_then(|_| fs::create_dir(&profile))
            .and_then(|_| fs::write(&input, docx_bytes))
            .map_err(|error| ArtifactRuntimeError::Staging(error.to_string()))?;

        let runtime_root = executable
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let request = ExecRequest::new(executable.to_string_lossy())
            .args([
                "--headless".to_string(),
                "--nologo".to_string(),
                "--nodefault".to_string(),
                "--nofirststartwizard".to_string(),
                format!("-env:UserInstallation={}", file_url(&profile)),
                "--convert-to".to_string(),
                "pdf".to_string(),
                "--outdir".to_string(),
                output_dir.to_string_lossy().to_string(),
                input.to_string_lossy().to_string(),
            ])
            .cwd(&staging.root)
            .runtime("libreoffice", vec![runtime_root])
            .requirements(ExecutionRequirements {
                read_paths: vec![input.clone()],
                write_paths: vec![output_dir.clone(), profile],
                network: Some(NetworkPolicy::Deny),
                ..Default::default()
            });
        let mut context = ExecutionContext::with_timeout(Duration::from_secs(90))
            .with_resource_limits(ResourceLimit {
                max_memory_bytes: Some(1024 * 1024 * 1024),
                max_output_bytes: Some(64 * 1024),
                ..Default::default()
            });
        context = context.with_cancel(cancel);
        let environment = LocalExecutionEnvironment::with_sandbox_config(
            &staging.root,
            isolated_artifact_sandbox(sandbox_config),
        );
        let result = environment
            .exec(request, context)
            .await
            .map_err(|error| ArtifactRuntimeError::LibreOffice(format!("{error:#}")))?;
        if !result.success {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(ArtifactRuntimeError::LibreOffice(format!(
                "process exited with {:?}: {}",
                result.exit_code,
                stderr.trim()
            )));
        }
        let pdf_path = output_dir.join("input.pdf");
        let pdf_metadata = fs::symlink_metadata(&pdf_path).map_err(|error| {
            ArtifactRuntimeError::LibreOffice(format!(
                "conversion did not produce {}: {error}",
                pdf_path.display()
            ))
        })?;
        if !pdf_metadata.file_type().is_file() {
            return Err(ArtifactRuntimeError::LibreOffice(format!(
                "conversion output {} is not a regular file",
                pdf_path.display()
            )));
        }
        let pdf_size = pdf_metadata.len();
        if pdf_size > MAX_ARTIFACT_INPUT_BYTES {
            return Err(ArtifactRuntimeError::LibreOffice(format!(
                "converted PDF is {pdf_size} bytes; limit is {MAX_ARTIFACT_INPUT_BYTES} bytes"
            )));
        }
        fs::read(&pdf_path).map_err(|error| {
            ArtifactRuntimeError::LibreOffice(format!(
                "conversion did not produce {}: {error}",
                pdf_path.display()
            ))
        })
    }
}

struct ArtifactSession {
    root: PathBuf,
}

impl ArtifactSession {
    fn new() -> Result<Self, ArtifactRuntimeError> {
        let root = env::temp_dir().join(format!("opentopia-artifact-{}", Uuid::new_v4()));
        fs::create_dir(&root).map_err(|error| ArtifactRuntimeError::Staging(error.to_string()))?;
        Ok(Self { root })
    }
}

impl Drop for ArtifactSession {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.root) {
            tracing::warn!(path = %self.root.display(), %error, "failed to remove artifact staging directory");
        }
    }
}

fn discover_libreoffice() -> Option<PathBuf> {
    if let Some(path) = env::var_os("OPENTOPIA_LIBREOFFICE_PATH") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let mut candidates = Vec::new();
    if cfg!(windows) {
        candidates.extend([
            PathBuf::from(r"C:\Program Files\LibreOffice\program\soffice.exe"),
            PathBuf::from(r"C:\Program Files (x86)\LibreOffice\program\soffice.exe"),
        ]);
    }
    candidates.extend(path_candidates(if cfg!(windows) {
        "soffice.exe"
    } else {
        "soffice"
    }));
    candidates.extend(path_candidates("libreoffice"));
    candidates.into_iter().find(|path| path.is_file())
}

fn isolated_artifact_sandbox(base: Option<LocalSandboxConfig>) -> LocalSandboxConfig {
    let mut config = base.unwrap_or_else(LocalSandboxConfig::from_env);
    config.enabled = true;
    config.mode = OsSandboxMode::Enforce;
    config.sandbox_mode = SandboxMode::WorkspaceWrite;
    config.network = NetworkPolicy::Deny;
    config.windows_backend = WindowsSandboxBackend::DedicatedUser;
    config.read_paths.clear();
    config.write_paths.clear();
    config.writable_roots.clear();
    config.approved_read_paths.clear();
    config.approved_write_paths.clear();
    config.sandbox_home = None;
    config
}

fn path_candidates(name: &str) -> Vec<PathBuf> {
    env::var_os("PATH")
        .map(|value| {
            env::split_paths(&value)
                .map(|root| root.join(name))
                .collect()
        })
        .unwrap_or_default()
}

fn file_url(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let escaped = normalized
        .replace('%', "%25")
        .replace(' ', "%20")
        .replace('#', "%23");
    if escaped.starts_with('/') {
        format!("file://{escaped}")
    } else {
        format!("file:///{escaped}")
    }
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

    #[test]
    fn file_url_is_stable_for_libreoffice_profiles() {
        let url = file_url(Path::new(r"C:\Temp\Open Topia#1"));
        assert_eq!(url, "file:///C:/Temp/Open%20Topia%231");
    }

    #[test]
    fn artifact_sandbox_is_fail_closed_even_from_unrestricted_sessions() {
        let config = isolated_artifact_sandbox(Some(LocalSandboxConfig::danger_full_access()));
        assert!(config.enabled);
        assert_eq!(config.mode, OsSandboxMode::Enforce);
        assert_eq!(config.sandbox_mode, SandboxMode::WorkspaceWrite);
        assert_eq!(config.network, NetworkPolicy::Deny);
        assert_eq!(config.windows_backend, WindowsSandboxBackend::DedicatedUser);
    }
}
