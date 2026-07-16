use crate::model::{Artifact, ArtifactStorage};
use crate::spreadsheet::{
    inspect_workbook, read_range, CellAddress, CellRange, InspectWorkbookRequest, ReadRangeRequest,
    SheetKind, SheetVisibility, SpreadsheetCell, SpreadsheetError, EXCEL_MAX_COLUMNS,
    EXCEL_MAX_ROWS,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;
use thiserror::Error;
use uuid::Uuid;

pub const MAX_PREVIEW_CONTENT_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewKind {
    Text,
    Image,
    Pdf,
    Spreadsheet,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewSource {
    Workspace,
    Artifact,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "source",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PreviewTarget {
    Workspace { path: PathBuf },
    Artifact { artifact_id: Uuid },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewDescriptor {
    pub id: String,
    pub source: PreviewSource,
    pub path: Option<PathBuf>,
    pub name: String,
    pub kind: PreviewKind,
    pub content_type: String,
    pub bytes: u64,
    pub readonly: bool,
    pub revision: String,
}

#[derive(Debug, Clone)]
pub enum PreviewContentSource {
    Path(PathBuf),
    Inline(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct ResolvedPreview {
    pub descriptor: PreviewDescriptor,
    pub content: PreviewContentSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewWorkbook {
    pub preview_id: String,
    pub bytes: u64,
    pub sheets: Vec<PreviewSheet>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSheet {
    pub name: String,
    pub kind: SheetKind,
    pub visibility: SheetVisibility,
    pub row_count: u32,
    pub column_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRangeRequest {
    pub sheet: String,
    pub start_row: u32,
    pub start_column: u32,
    pub row_count: u32,
    pub column_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRange {
    pub preview_id: String,
    pub sheet: String,
    pub range: CellRange,
    pub rows: Vec<Vec<SpreadsheetCell>>,
}

#[derive(Debug, Error)]
pub enum PreviewError {
    #[error("invalid preview id: {0}")]
    InvalidPreviewId(String),
    #[error("workspace root was not found: {0}")]
    WorkspaceRootNotFound(PathBuf),
    #[error("workspace path cannot contain parent-directory components")]
    ParentDirectoryNotAllowed,
    #[error("workspace path was not found: {0}")]
    PathNotFound(PathBuf),
    #[error("path is outside the workspace: {0}")]
    OutsideWorkspace(PathBuf),
    #[error("preview path is not a file: {0}")]
    NotAFile(PathBuf),
    #[error("artifact {artifact_id} does not belong to thread {thread_id}")]
    ArtifactThreadMismatch { artifact_id: Uuid, thread_id: Uuid },
    #[error("preview content is {actual_bytes} bytes; limit is {limit_bytes} bytes")]
    ContentTooLarge { actual_bytes: u64, limit_bytes: u64 },
    #[error("preview {0} is not an XLSX spreadsheet")]
    NotSpreadsheet(String),
    #[error("inline spreadsheet previews are not supported")]
    InlineSpreadsheetUnsupported,
    #[error("invalid spreadsheet preview range: {0}")]
    InvalidRange(&'static str),
    #[error("failed to read preview file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Spreadsheet(#[from] SpreadsheetError),
}

pub fn encode_preview_id(target: &PreviewTarget) -> String {
    match target {
        PreviewTarget::Workspace { path } => {
            let path = path.to_string_lossy();
            format!("workspace.{}", URL_SAFE_NO_PAD.encode(path.as_bytes()))
        }
        PreviewTarget::Artifact { artifact_id } => format!("artifact.{artifact_id}"),
    }
}

pub fn decode_preview_id(id: &str) -> Result<PreviewTarget, PreviewError> {
    if let Some(encoded) = id.strip_prefix("workspace.") {
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| PreviewError::InvalidPreviewId(id.to_string()))?;
        let path =
            String::from_utf8(bytes).map_err(|_| PreviewError::InvalidPreviewId(id.to_string()))?;
        if path.trim().is_empty() {
            return Err(PreviewError::InvalidPreviewId(id.to_string()));
        }
        return Ok(PreviewTarget::Workspace {
            path: PathBuf::from(path),
        });
    }
    if let Some(raw_id) = id.strip_prefix("artifact.") {
        let artifact_id =
            Uuid::parse_str(raw_id).map_err(|_| PreviewError::InvalidPreviewId(id.to_string()))?;
        return Ok(PreviewTarget::Artifact { artifact_id });
    }
    Err(PreviewError::InvalidPreviewId(id.to_string()))
}

pub fn resolve_workspace_preview(
    workspace_root: &Path,
    requested: &Path,
) -> Result<ResolvedPreview, PreviewError> {
    let root = workspace_root
        .canonicalize()
        .map_err(|_| PreviewError::WorkspaceRootNotFound(workspace_root.to_path_buf()))?;
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(PreviewError::ParentDirectoryNotAllowed);
    }
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let resolved = candidate
        .canonicalize()
        .map_err(|_| PreviewError::PathNotFound(candidate.clone()))?;
    if !resolved.starts_with(&root) {
        return Err(PreviewError::OutsideWorkspace(resolved));
    }

    let metadata = file_metadata(&resolved)?;
    let relative_path = resolved
        .strip_prefix(&root)
        .expect("workspace boundary checked")
        .to_path_buf();
    let content_type = infer_content_type(&resolved, None);
    let target = PreviewTarget::Workspace {
        path: relative_path.clone(),
    };
    let descriptor = PreviewDescriptor {
        id: encode_preview_id(&target),
        source: PreviewSource::Workspace,
        path: Some(relative_path),
        name: file_name(&resolved),
        kind: classify_preview(&content_type, &resolved),
        content_type,
        bytes: metadata.len(),
        readonly: true,
        revision: file_revision("w", &metadata),
    };
    Ok(ResolvedPreview {
        descriptor,
        content: PreviewContentSource::Path(resolved),
    })
}

pub fn resolve_artifact_preview(
    thread_id: Uuid,
    workspace_root: &Path,
    artifact: &Artifact,
) -> Result<ResolvedPreview, PreviewError> {
    if artifact.thread_id != thread_id {
        return Err(PreviewError::ArtifactThreadMismatch {
            artifact_id: artifact.id,
            thread_id,
        });
    }

    let target = PreviewTarget::Artifact {
        artifact_id: artifact.id,
    };
    let (path, name, bytes, content_type, content, revision) = match &artifact.storage {
        ArtifactStorage::Inline { content } => {
            let bytes = content.as_bytes().to_vec();
            let byte_len = bytes.len() as u64;
            let name = artifact_display_name(artifact, None);
            let content_type = infer_content_type(Path::new(&name), Some(&artifact.content_type));
            (
                None,
                name,
                byte_len,
                content_type,
                PreviewContentSource::Inline(bytes),
                format!(
                    "a-{}-{:x}-{:x}",
                    artifact.id,
                    byte_len,
                    artifact.created_at.timestamp_millis().max(0)
                ),
            )
        }
        ArtifactStorage::Path { path } => {
            let candidate = if path.is_absolute() {
                path.clone()
            } else {
                workspace_root.join(path)
            };
            let resolved = candidate
                .canonicalize()
                .map_err(|_| PreviewError::PathNotFound(candidate.clone()))?;
            let metadata = file_metadata(&resolved)?;
            let name = artifact_display_name(artifact, Some(&resolved));
            let content_type = infer_content_type(&resolved, Some(&artifact.content_type));
            (
                Some(resolved.clone()),
                name,
                metadata.len(),
                content_type,
                PreviewContentSource::Path(resolved),
                file_revision(&format!("a-{}", artifact.id), &metadata),
            )
        }
    };
    let descriptor = PreviewDescriptor {
        id: encode_preview_id(&target),
        source: PreviewSource::Artifact,
        path,
        name,
        kind: classify_preview(&content_type, descriptor_path(&content)),
        content_type,
        bytes,
        readonly: true,
        revision,
    };
    Ok(ResolvedPreview {
        descriptor,
        content,
    })
}

pub fn read_preview_content(
    preview: &ResolvedPreview,
    max_bytes: u64,
) -> Result<Vec<u8>, PreviewError> {
    if preview.descriptor.bytes > max_bytes {
        return Err(PreviewError::ContentTooLarge {
            actual_bytes: preview.descriptor.bytes,
            limit_bytes: max_bytes,
        });
    }
    let bytes = match &preview.content {
        PreviewContentSource::Path(path) => {
            std::fs::read(path).map_err(|source| PreviewError::Io {
                path: path.clone(),
                source,
            })?
        }
        PreviewContentSource::Inline(bytes) => bytes.clone(),
    };
    if bytes.len() as u64 > max_bytes {
        return Err(PreviewError::ContentTooLarge {
            actual_bytes: bytes.len() as u64,
            limit_bytes: max_bytes,
        });
    }
    Ok(bytes)
}

pub fn preview_workbook(preview: &ResolvedPreview) -> Result<PreviewWorkbook, PreviewError> {
    let path = spreadsheet_path(preview)?;
    let result = inspect_workbook(&InspectWorkbookRequest {
        path: path.to_path_buf(),
    })?;
    let sheets = result
        .sheets
        .into_iter()
        .filter(|sheet| sheet.sheet.kind == SheetKind::Worksheet)
        .map(|sheet| {
            let (row_count, column_count) = sheet
                .used_range
                .map(|range| {
                    (
                        range.end.row.saturating_add(1),
                        range.end.column.saturating_add(1),
                    )
                })
                .unwrap_or((1, 1));
            PreviewSheet {
                name: sheet.sheet.name,
                kind: sheet.sheet.kind,
                visibility: sheet.sheet.visibility,
                row_count,
                column_count,
            }
        })
        .collect();
    Ok(PreviewWorkbook {
        preview_id: preview.descriptor.id.clone(),
        bytes: result.file_size_bytes,
        sheets,
    })
}

pub fn preview_spreadsheet_range(
    preview: &ResolvedPreview,
    request: PreviewRangeRequest,
) -> Result<PreviewRange, PreviewError> {
    if request.sheet.trim().is_empty() {
        return Err(PreviewError::InvalidRange("sheet cannot be empty"));
    }
    if request.row_count == 0 || request.column_count == 0 {
        return Err(PreviewError::InvalidRange(
            "rowCount and columnCount must be greater than zero",
        ));
    }
    let end_row = request
        .start_row
        .checked_add(request.row_count - 1)
        .ok_or(PreviewError::InvalidRange("row range overflow"))?;
    let end_column = request
        .start_column
        .checked_add(request.column_count - 1)
        .ok_or(PreviewError::InvalidRange("column range overflow"))?;
    if end_row >= EXCEL_MAX_ROWS || end_column >= EXCEL_MAX_COLUMNS {
        return Err(PreviewError::InvalidRange(
            "range exceeds XLSX row or column bounds",
        ));
    }

    let path = spreadsheet_path(preview)?;
    let range = CellRange {
        start: CellAddress {
            row: request.start_row,
            column: request.start_column,
        },
        end: CellAddress {
            row: end_row,
            column: end_column,
        },
    };
    let result = read_range(&ReadRangeRequest {
        path: path.to_path_buf(),
        sheet: request.sheet.clone(),
        range,
    })?;
    Ok(PreviewRange {
        preview_id: preview.descriptor.id.clone(),
        sheet: result.sheet,
        range: result.range,
        rows: result.rows,
    })
}

fn spreadsheet_path(preview: &ResolvedPreview) -> Result<&Path, PreviewError> {
    if preview.descriptor.kind != PreviewKind::Spreadsheet {
        return Err(PreviewError::NotSpreadsheet(preview.descriptor.id.clone()));
    }
    match &preview.content {
        PreviewContentSource::Path(path) => Ok(path),
        PreviewContentSource::Inline(_) => Err(PreviewError::InlineSpreadsheetUnsupported),
    }
}

fn file_metadata(path: &Path) -> Result<std::fs::Metadata, PreviewError> {
    let metadata = std::fs::metadata(path).map_err(|source| PreviewError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(PreviewError::NotAFile(path.to_path_buf()));
    }
    Ok(metadata)
}

fn file_revision(prefix: &str, metadata: &std::fs::Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{:x}-{modified:x}", metadata.len())
}

fn artifact_display_name(artifact: &Artifact, path: Option<&Path>) -> String {
    artifact
        .metadata
        .get("name")
        .or_else(|| artifact.metadata.get("fileName"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| path.map(file_name))
        .unwrap_or_else(|| format!("artifact-{}", artifact.id))
}

fn descriptor_path(content: &PreviewContentSource) -> &Path {
    match content {
        PreviewContentSource::Path(path) => path,
        PreviewContentSource::Inline(_) => Path::new(""),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

fn infer_content_type(path: &Path, declared: Option<&str>) -> String {
    let declared = declared.map(str::trim).filter(|value| !value.is_empty());
    if let Some(content_type) = declared {
        if content_type != "application/octet-stream" {
            return content_type.to_string();
        }
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let inferred = match extension.as_str() {
        "txt" | "log" => "text/plain; charset=utf-8",
        "md" | "markdown" => "text/markdown; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "yaml" | "yml" => "application/yaml; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ if is_source_extension(&extension) => "text/plain; charset=utf-8",
        _ => return declared.unwrap_or("application/octet-stream").to_string(),
    };
    inferred.to_string()
}

fn classify_preview(content_type: &str, path: &Path) -> PreviewKind {
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();
    if media_type == "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        || path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("xlsx"))
    {
        PreviewKind::Spreadsheet
    } else if media_type == "application/pdf" {
        PreviewKind::Pdf
    } else if media_type.starts_with("image/") {
        PreviewKind::Image
    } else if media_type.starts_with("text/")
        || matches!(
            media_type.as_str(),
            "application/json" | "application/xml" | "application/yaml"
        )
    {
        PreviewKind::Text
    } else {
        PreviewKind::Unsupported
    }
}

fn is_source_extension(extension: &str) -> bool {
    matches!(
        extension,
        "rs" | "toml"
            | "lock"
            | "ts"
            | "tsx"
            | "jsx"
            | "vue"
            | "svelte"
            | "py"
            | "go"
            | "java"
            | "kt"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "hpp"
            | "cs"
            | "rb"
            | "php"
            | "sql"
            | "sh"
            | "bash"
            | "zsh"
            | "ps1"
            | "bat"
            | "cmd"
            | "ini"
            | "conf"
            | "env"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spreadsheet::{
        write_workbook, CellUpdate, SheetWriteRequest, SpreadsheetCellInput, WriteWorkbookRequest,
    };
    use chrono::Utc;
    use serde_json::json;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("opentopia-preview-{}", Uuid::new_v4()));
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
    fn workspace_preview_round_trips_id_and_preserves_binary_content() {
        let directory = TestDirectory::new();
        let bytes = [0_u8, 159, 146, 150, 255];
        std::fs::write(directory.path().join("sample.bin"), bytes).expect("write binary file");

        let preview = resolve_workspace_preview(directory.path(), Path::new("sample.bin"))
            .expect("resolve preview");
        assert_eq!(preview.descriptor.kind, PreviewKind::Unsupported);
        assert_eq!(preview.descriptor.bytes, bytes.len() as u64);
        assert_eq!(
            decode_preview_id(&preview.descriptor.id).expect("decode preview id"),
            PreviewTarget::Workspace {
                path: PathBuf::from("sample.bin")
            }
        );
        assert_eq!(
            read_preview_content(&preview, MAX_PREVIEW_CONTENT_BYTES).expect("read preview"),
            bytes
        );
    }

    #[test]
    fn workspace_preview_rejects_parent_escape() {
        let directory = TestDirectory::new();
        let error = resolve_workspace_preview(directory.path(), Path::new("../outside.txt"))
            .expect_err("parent traversal must fail");
        assert!(matches!(error, PreviewError::ParentDirectoryNotAllowed));
    }

    #[test]
    fn artifact_preview_enforces_thread_ownership() {
        let directory = TestDirectory::new();
        let owner = Uuid::new_v4();
        let artifact = Artifact::inline(
            owner,
            "text",
            "text/plain; charset=utf-8",
            "hello",
            json!({"name": "answer.txt"}),
        );
        let other_thread = Uuid::new_v4();

        let error = resolve_artifact_preview(other_thread, directory.path(), &artifact)
            .expect_err("cross-thread artifact must fail");
        assert!(matches!(
            error,
            PreviewError::ArtifactThreadMismatch { thread_id, .. } if thread_id == other_thread
        ));
    }

    #[test]
    fn spreadsheet_preview_reuses_bounded_workbook_and_range_readers() {
        let directory = TestDirectory::new();
        let workbook_path = directory.path().join("report.xlsx");
        write_workbook(&WriteWorkbookRequest {
            source: None,
            output: workbook_path.clone(),
            sheets: vec![SheetWriteRequest {
                name: "Data".to_string(),
                visibility: None,
                cells: vec![CellUpdate {
                    address: CellAddress { row: 0, column: 0 },
                    value: SpreadsheetCellInput::String("OpenTopia".to_string()),
                }],
            }],
        })
        .expect("write workbook");

        let preview = resolve_workspace_preview(directory.path(), Path::new("report.xlsx"))
            .expect("resolve workbook preview");
        let workbook = preview_workbook(&preview).expect("read workbook metadata");
        assert_eq!(workbook.sheets.len(), 1);
        assert_eq!(workbook.sheets[0].name, "Data");
        assert_eq!(workbook.sheets[0].row_count, 1);
        assert_eq!(workbook.sheets[0].column_count, 1);

        let range = preview_spreadsheet_range(
            &preview,
            PreviewRangeRequest {
                sheet: "Data".to_string(),
                start_row: 0,
                start_column: 0,
                row_count: 1,
                column_count: 1,
            },
        )
        .expect("read workbook range");
        assert_eq!(range.rows.len(), 1);
        assert_eq!(range.rows[0].len(), 1);
        assert_eq!(
            range.rows[0][0].value,
            crate::spreadsheet::SpreadsheetCellValue::String("OpenTopia".to_string())
        );
    }

    #[test]
    fn path_artifact_uses_file_revision_and_declared_name() {
        let directory = TestDirectory::new();
        let path = directory.path().join("image.png");
        std::fs::write(&path, b"png").expect("write artifact file");
        let thread_id = Uuid::new_v4();
        let artifact = Artifact {
            id: Uuid::new_v4(),
            thread_id,
            kind: "image".to_string(),
            content_type: "image/png".to_string(),
            storage: ArtifactStorage::Path { path: path.clone() },
            bytes: 999,
            created_at: Utc::now(),
            metadata: json!({"name": "chart.png"}),
        };

        let preview = resolve_artifact_preview(thread_id, directory.path(), &artifact)
            .expect("resolve artifact preview");
        assert_eq!(preview.descriptor.name, "chart.png");
        assert_eq!(preview.descriptor.bytes, 3);
        assert_eq!(preview.descriptor.kind, PreviewKind::Image);
        assert!(preview.descriptor.revision.starts_with("a-"));
    }
}
