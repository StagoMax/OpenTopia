use crate::model::{Artifact, ArtifactStorage, ContextSourceRef};
use crate::spreadsheet::{
    inspect_workbook, read_range_for_display, CellAddress, CellRange, InspectWorkbookRequest,
    ReadRangeRequest, SheetKind, SheetVisibility, SpreadsheetCellValue, SpreadsheetError,
    SpreadsheetFileFormat, EXCEL_MAX_COLUMNS, EXCEL_MAX_ROWS,
    MAX_INPUT_FILE_BYTES as MAX_SPREADSHEET_INPUT_BYTES, MAX_READ_CELLS, MAX_READ_COLUMNS,
    MAX_READ_ROWS,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;
use thiserror::Error;
use uuid::Uuid;

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

pub const MAX_PREVIEW_CONTENT_BYTES: u64 = 100 * 1024 * 1024;
const DELIMITED_PREVIEW_SHEET: &str = "Data";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewKind {
    Text,
    Image,
    Pdf,
    Document,
    Spreadsheet,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreviewSource {
    Workspace,
    Local,
    Artifact,
    Attachment,
}

/// Operations granted to a resolved resource. Renderers consume these
/// capabilities instead of inferring authority from where a file came from.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewCapabilities {
    pub read: bool,
    pub write: bool,
    pub watch: bool,
    pub range_read: bool,
    pub open_external: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "source",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PreviewTarget {
    Workspace { path: PathBuf },
    Local { resource_id: Uuid },
    Artifact { artifact_id: Uuid },
    Attachment { attachment_id: Uuid },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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
    #[serde(default)]
    pub capabilities: PreviewCapabilities,
    pub revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler_id: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewWorkbook {
    pub preview_id: String,
    pub bytes: u64,
    pub sheets: Vec<PreviewSheet>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRange {
    pub preview_id: String,
    pub sheet: String,
    pub range: CellRange,
    pub rows: Vec<Vec<PreviewCell>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewCell {
    pub value: SpreadsheetCellValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formatted: Option<String>,
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
    #[error("preview resource is read-only: {0}")]
    ReadOnly(String),
    #[error("preview resource changed (expected {expected}, current {actual})")]
    RevisionConflict { expected: String, actual: String },
    #[error("artifact {artifact_id} does not belong to thread {thread_id}")]
    ArtifactThreadMismatch { artifact_id: Uuid, thread_id: Uuid },
    #[error("preview content is {actual_bytes} bytes; limit is {limit_bytes} bytes")]
    ContentTooLarge { actual_bytes: u64, limit_bytes: u64 },
    #[error("preview {0} is not a supported spreadsheet")]
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
    #[error("failed to parse delimited preview file {path}: {source}")]
    Delimited {
        path: PathBuf,
        #[source]
        source: csv::Error,
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
        PreviewTarget::Local { resource_id } => format!("local.{resource_id}"),
        PreviewTarget::Artifact { artifact_id } => format!("artifact.{artifact_id}"),
        PreviewTarget::Attachment { attachment_id } => {
            format!("attachment.{attachment_id}")
        }
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
    if let Some(raw_id) = id.strip_prefix("local.") {
        let resource_id =
            Uuid::parse_str(raw_id).map_err(|_| PreviewError::InvalidPreviewId(id.to_string()))?;
        return Ok(PreviewTarget::Local { resource_id });
    }
    if let Some(raw_id) = id.strip_prefix("attachment.") {
        let attachment_id =
            Uuid::parse_str(raw_id).map_err(|_| PreviewError::InvalidPreviewId(id.to_string()))?;
        return Ok(PreviewTarget::Attachment { attachment_id });
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
    let kind = classify_preview(&content_type, &resolved);
    let capabilities = file_capabilities(&metadata, kind);
    let target = PreviewTarget::Workspace {
        path: relative_path.clone(),
    };
    let descriptor = PreviewDescriptor {
        id: encode_preview_id(&target),
        source: PreviewSource::Workspace,
        path: Some(relative_path),
        name: file_name(&resolved),
        kind,
        content_type,
        bytes: metadata.len(),
        readonly: !capabilities.write,
        capabilities,
        revision: file_revision("w", &metadata),
        handler_id: None,
    };
    Ok(ResolvedPreview {
        descriptor,
        content: PreviewContentSource::Path(resolved),
    })
}

/// Resolves a user-opened local file. The raw path is accepted only at the
/// resource-registration boundary; subsequent operations use `resource_id`.
pub fn resolve_local_preview(
    resource_id: Uuid,
    requested: &Path,
) -> Result<ResolvedPreview, PreviewError> {
    let resolved = requested
        .canonicalize()
        .map_err(|_| PreviewError::PathNotFound(requested.to_path_buf()))?;
    let metadata = file_metadata(&resolved)?;
    let content_type = infer_content_type(&resolved, None);
    let kind = classify_preview(&content_type, &resolved);
    let capabilities = file_capabilities(&metadata, kind);
    let target = PreviewTarget::Local { resource_id };
    let descriptor = PreviewDescriptor {
        id: encode_preview_id(&target),
        source: PreviewSource::Local,
        path: Some(resolved.clone()),
        name: file_name(&resolved),
        kind,
        content_type,
        bytes: metadata.len(),
        readonly: !capabilities.write,
        capabilities,
        revision: file_revision(&format!("l-{resource_id}"), &metadata),
        handler_id: None,
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
    let kind = classify_preview(&content_type, Path::new(&name));
    let open_external = path.is_some();
    let descriptor = PreviewDescriptor {
        id: encode_preview_id(&target),
        source: PreviewSource::Artifact,
        path,
        name,
        kind,
        content_type,
        bytes,
        readonly: true,
        capabilities: PreviewCapabilities {
            read: true,
            open_external,
            range_read: kind == PreviewKind::Spreadsheet,
            ..PreviewCapabilities::default()
        },
        revision,
        handler_id: None,
    };
    Ok(ResolvedPreview {
        descriptor,
        content,
    })
}

/// Resolves a user-selected context source that was persisted on a message.
///
/// Unlike workspace previews, attachments may intentionally live outside the
/// workspace. Callers must first look up the opaque attachment ID inside the
/// route thread; accepting a path directly here would bypass that ownership
/// check.
pub fn resolve_attachment_preview(
    attachment: &ContextSourceRef,
) -> Result<ResolvedPreview, PreviewError> {
    let resolved = attachment
        .path
        .canonicalize()
        .map_err(|_| PreviewError::PathNotFound(attachment.path.clone()))?;
    let metadata = file_metadata(&resolved)?;
    let content_type = infer_content_type(&resolved, Some(&attachment.content_type));
    let kind = classify_preview(&content_type, Path::new(&attachment.name));
    let target = PreviewTarget::Attachment {
        attachment_id: attachment.id,
    };
    let descriptor = PreviewDescriptor {
        id: encode_preview_id(&target),
        source: PreviewSource::Attachment,
        path: Some(resolved.clone()),
        name: attachment.name.clone(),
        kind,
        content_type,
        bytes: metadata.len(),
        readonly: true,
        capabilities: PreviewCapabilities {
            read: true,
            range_read: kind == PreviewKind::Spreadsheet,
            open_external: true,
            ..PreviewCapabilities::default()
        },
        revision: file_revision(&format!("u-{}", attachment.id), &metadata),
        handler_id: None,
    };
    Ok(ResolvedPreview {
        descriptor,
        content: PreviewContentSource::Path(resolved),
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

/// Commits edited content using optimistic concurrency and a same-directory
/// atomic replacement. A UTF-8 BOM already present in the file is preserved.
pub fn write_preview_content(
    preview: &ResolvedPreview,
    expected_revision: &str,
    content: &[u8],
    max_bytes: u64,
) -> Result<(), PreviewError> {
    if !preview.descriptor.capabilities.write {
        return Err(PreviewError::ReadOnly(preview.descriptor.id.clone()));
    }
    if content.len() as u64 > max_bytes {
        return Err(PreviewError::ContentTooLarge {
            actual_bytes: content.len() as u64,
            limit_bytes: max_bytes,
        });
    }
    let path = match &preview.content {
        PreviewContentSource::Path(path) => path,
        PreviewContentSource::Inline(_) => {
            return Err(PreviewError::ReadOnly(preview.descriptor.id.clone()))
        }
    };
    let metadata = file_metadata(path)?;
    let actual_revision = file_revision(revision_prefix(&preview.descriptor.revision), &metadata);
    if actual_revision != expected_revision {
        return Err(PreviewError::RevisionConflict {
            expected: expected_revision.to_string(),
            actual: actual_revision,
        });
    }
    let preserve_utf8_bom = std::fs::File::open(path)
        .and_then(|mut file| {
            let mut bom = [0_u8; 3];
            file.read_exact(&mut bom).map(|_| bom == [0xef, 0xbb, 0xbf])
        })
        .unwrap_or(false);
    let mut bytes = Vec::with_capacity(content.len() + usize::from(preserve_utf8_bom) * 3);
    if preserve_utf8_bom && !content.starts_with(&[0xef, 0xbb, 0xbf]) {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(content);
    atomic_replace(path, &bytes)
}

pub fn preview_workbook(preview: &ResolvedPreview) -> Result<PreviewWorkbook, PreviewError> {
    if let Some(delimiter) = delimited_preview_delimiter(preview) {
        return preview_delimited_workbook(preview, delimiter);
    }
    let path = spreadsheet_path(preview)?;
    let result = inspect_workbook(&InspectWorkbookRequest {
        path: path.as_path().to_path_buf(),
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
            "range exceeds spreadsheet row or column bounds",
        ));
    }
    if u64::from(request.row_count) > MAX_READ_ROWS
        || u64::from(request.column_count) > MAX_READ_COLUMNS
        || u64::from(request.row_count) * u64::from(request.column_count) > MAX_READ_CELLS
    {
        return Err(PreviewError::InvalidRange(
            "range exceeds spreadsheet preview limits",
        ));
    }

    if let Some(delimiter) = delimited_preview_delimiter(preview) {
        return preview_delimited_range(preview, delimiter, request);
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
    let result = read_range_for_display(&ReadRangeRequest {
        path: path.as_path().to_path_buf(),
        sheet: request.sheet.clone(),
        range,
    })?;
    Ok(PreviewRange {
        preview_id: preview.descriptor.id.clone(),
        sheet: result.sheet,
        range: result.range,
        rows: result
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|cell| PreviewCell {
                        value: cell.value,
                        formula: cell.formula,
                        formatted: cell.formatted,
                    })
                    .collect()
            })
            .collect(),
    })
}

fn preview_delimited_workbook(
    preview: &ResolvedPreview,
    delimiter: u8,
) -> Result<PreviewWorkbook, PreviewError> {
    ensure_delimited_preview_size(preview)?;
    let mut reader = delimited_reader(preview, delimiter)?;
    let error_path = preview_error_path(preview);
    let mut row_count = 0_u32;
    let mut column_count = 0_u32;
    for record in reader.byte_records() {
        let record = record.map_err(|source| PreviewError::Delimited {
            path: error_path.clone(),
            source,
        })?;
        row_count = row_count.checked_add(1).ok_or(PreviewError::InvalidRange(
            "delimited file has too many rows",
        ))?;
        column_count = column_count.max(
            u32::try_from(record.len())
                .map_err(|_| PreviewError::InvalidRange("delimited file has too many columns"))?,
        );
        if row_count > EXCEL_MAX_ROWS || column_count > EXCEL_MAX_COLUMNS {
            return Err(PreviewError::InvalidRange(
                "delimited file exceeds spreadsheet preview bounds",
            ));
        }
    }
    Ok(PreviewWorkbook {
        preview_id: preview.descriptor.id.clone(),
        bytes: preview.descriptor.bytes,
        sheets: vec![PreviewSheet {
            name: DELIMITED_PREVIEW_SHEET.to_string(),
            kind: SheetKind::Worksheet,
            visibility: SheetVisibility::Visible,
            row_count: row_count.max(1),
            column_count: column_count.max(1),
        }],
    })
}

fn preview_delimited_range(
    preview: &ResolvedPreview,
    delimiter: u8,
    request: PreviewRangeRequest,
) -> Result<PreviewRange, PreviewError> {
    if request.sheet != DELIMITED_PREVIEW_SHEET {
        return Err(PreviewError::InvalidRange(
            "delimited preview sheet was not found",
        ));
    }
    ensure_delimited_preview_size(preview)?;
    let range = CellRange {
        start: CellAddress {
            row: request.start_row,
            column: request.start_column,
        },
        end: CellAddress {
            row: request.start_row + request.row_count - 1,
            column: request.start_column + request.column_count - 1,
        },
    };
    let mut reader = delimited_reader(preview, delimiter)?;
    let error_path = preview_error_path(preview);
    let mut rows = Vec::with_capacity(request.row_count as usize);
    for (row_index, record) in reader.byte_records().enumerate() {
        let row_index = u32::try_from(row_index)
            .map_err(|_| PreviewError::InvalidRange("delimited file has too many rows"))?;
        if row_index < request.start_row {
            continue;
        }
        if row_index > range.end.row {
            break;
        }
        let record = record.map_err(|source| PreviewError::Delimited {
            path: error_path.clone(),
            source,
        })?;
        let mut cells = Vec::with_capacity(request.column_count as usize);
        for column in request.start_column..=range.end.column {
            let value = record
                .get(column as usize)
                .map(|field| delimited_cell_value(field, row_index, column))
                .unwrap_or(SpreadsheetCellValue::Empty);
            cells.push(PreviewCell {
                value,
                formula: None,
                formatted: None,
            });
        }
        rows.push(cells);
    }
    while rows.len() < request.row_count as usize {
        rows.push(
            (0..request.column_count)
                .map(|_| PreviewCell {
                    value: SpreadsheetCellValue::Empty,
                    formula: None,
                    formatted: None,
                })
                .collect(),
        );
    }
    Ok(PreviewRange {
        preview_id: preview.descriptor.id.clone(),
        sheet: DELIMITED_PREVIEW_SHEET.to_string(),
        range,
        rows,
    })
}

fn delimited_reader<'a>(
    preview: &'a ResolvedPreview,
    delimiter: u8,
) -> Result<csv::Reader<Box<dyn Read + 'a>>, PreviewError> {
    let source: Box<dyn Read + 'a> = match &preview.content {
        PreviewContentSource::Path(path) => Box::new(std::fs::File::open(path).map_err(
            |source| PreviewError::Io {
                path: path.clone(),
                source,
            },
        )?),
        PreviewContentSource::Inline(bytes) => Box::new(Cursor::new(bytes.as_slice())),
    };
    Ok(crate::delimited::byte_reader(source, delimiter))
}

fn delimited_cell_value(field: &[u8], row: u32, column: u32) -> SpreadsheetCellValue {
    let value = crate::delimited::decode_field(field, row == 0 && column == 0, false);
    if value.is_empty() {
        SpreadsheetCellValue::Empty
    } else {
        SpreadsheetCellValue::String(value)
    }
}

fn ensure_delimited_preview_size(preview: &ResolvedPreview) -> Result<(), PreviewError> {
    let bytes = preview.descriptor.bytes;
    if bytes > MAX_PREVIEW_CONTENT_BYTES {
        return Err(PreviewError::ContentTooLarge {
            actual_bytes: bytes,
            limit_bytes: MAX_PREVIEW_CONTENT_BYTES,
        });
    }
    Ok(())
}

fn preview_error_path(preview: &ResolvedPreview) -> PathBuf {
    preview
        .descriptor
        .path
        .clone()
        .unwrap_or_else(|| PathBuf::from(&preview.descriptor.name))
}

fn delimited_preview_delimiter(preview: &ResolvedPreview) -> Option<u8> {
    let media_type = preview
        .descriptor
        .content_type
        .split(';')
        .next()
        .unwrap_or(&preview.descriptor.content_type)
        .trim()
        .to_ascii_lowercase();
    let extension = Path::new(&preview.descriptor.name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if media_type == "text/tab-separated-values" || matches!(extension.as_str(), "tsv" | "tab") {
        Some(b'\t')
    } else if media_type == "text/csv" || extension == "csv" {
        Some(b',')
    } else {
        None
    }
}

enum SpreadsheetPreviewPath<'a> {
    Borrowed(&'a Path),
    Staged(PathBuf),
}

impl SpreadsheetPreviewPath<'_> {
    fn as_path(&self) -> &Path {
        match self {
            Self::Borrowed(path) => path,
            Self::Staged(path) => path,
        }
    }
}

impl Drop for SpreadsheetPreviewPath<'_> {
    fn drop(&mut self) {
        if let Self::Staged(path) = self {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn spreadsheet_path(preview: &ResolvedPreview) -> Result<SpreadsheetPreviewPath<'_>, PreviewError> {
    if preview.descriptor.kind != PreviewKind::Spreadsheet {
        return Err(PreviewError::NotSpreadsheet(preview.descriptor.id.clone()));
    }
    match &preview.content {
        PreviewContentSource::Path(path) => {
            if SpreadsheetFileFormat::from_path(path)
                .is_some_and(SpreadsheetFileFormat::is_workbook)
            {
                return Ok(SpreadsheetPreviewPath::Borrowed(path));
            }
            if preview.descriptor.bytes > MAX_SPREADSHEET_INPUT_BYTES {
                return Err(PreviewError::Spreadsheet(SpreadsheetError::FileTooLarge {
                    path: path.clone(),
                    actual_bytes: preview.descriptor.bytes,
                    limit_bytes: MAX_SPREADSHEET_INPUT_BYTES,
                }));
            }
            let format = SpreadsheetFileFormat::from_path(Path::new(&preview.descriptor.name))
                .or_else(|| {
                    SpreadsheetFileFormat::from_content_type(&preview.descriptor.content_type)
                })
                .filter(|format| format.is_workbook())
                .ok_or_else(|| {
                    PreviewError::Spreadsheet(SpreadsheetError::UnsupportedFormat {
                        path: path.clone(),
                        extension: path
                            .extension()
                            .and_then(|extension| extension.to_str())
                            .map(str::to_string),
                    })
                })?;
            let staged = SpreadsheetPreviewPath::Staged(std::env::temp_dir().join(format!(
                "opentopia-preview-{}.{}",
                Uuid::new_v4(),
                format.extension()
            )));
            std::fs::copy(path, staged.as_path()).map_err(|source| PreviewError::Io {
                path: path.clone(),
                source,
            })?;
            Ok(staged)
        }
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

fn file_capabilities(metadata: &std::fs::Metadata, kind: PreviewKind) -> PreviewCapabilities {
    PreviewCapabilities {
        read: true,
        write: !metadata.permissions().readonly(),
        watch: true,
        range_read: kind == PreviewKind::Spreadsheet,
        open_external: true,
    }
}

fn atomic_replace(path: &Path, content: &[u8]) -> Result<(), PreviewError> {
    let parent = path.parent().ok_or_else(|| PreviewError::Io {
        path: path.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "preview path has no parent directory",
        ),
    })?;
    let name = path
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_else(|| "resource".into());
    let temporary = parent.join(format!(".{name}.opentopia-{}.tmp", Uuid::new_v4()));
    let write_result = (|| -> Result<(), std::io::Error> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(content)?;
        file.sync_all()
    })();
    if let Err(source) = write_result {
        let _ = std::fs::remove_file(&temporary);
        return Err(PreviewError::Io {
            path: temporary,
            source,
        });
    }
    if let Err(source) = publish_atomic_replacement(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(PreviewError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

#[cfg(not(windows))]
fn publish_atomic_replacement(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(temporary, destination)
}

#[cfg(windows)]
fn publish_atomic_replacement(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    let temporary_wide = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let moved = unsafe {
        MoveFileExW(
            temporary_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
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

fn revision_prefix(revision: &str) -> &str {
    revision
        .rsplit_once('-')
        .and_then(|(without_modified, _)| without_modified.rsplit_once('-'))
        .map(|(prefix, _)| prefix)
        .unwrap_or(revision)
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
        _ if SpreadsheetFileFormat::from_extension(&extension).is_some() => {
            SpreadsheetFileFormat::from_extension(&extension)
                .expect("spreadsheet extension was checked")
                .content_type()
        }
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
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
    if media_type == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        || path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("docx"))
    {
        PreviewKind::Document
    } else if SpreadsheetFileFormat::from_content_type(&media_type).is_some()
        || path
            .extension()
            .and_then(|value| value.to_str())
            .and_then(SpreadsheetFileFormat::from_extension)
            .is_some()
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
    use crate::context_sources::ContextSourceKind;
    use crate::spreadsheet::{
        write_workbook, CellUpdate, SheetWriteRequest, SpreadsheetCellInput, WriteWorkbookRequest,
    };
    use chrono::Utc;
    use rust_xlsxwriter::{Format, Workbook};
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
    fn local_preview_uses_an_opaque_identity_and_can_write_with_a_revision() {
        let directory = TestDirectory::new();
        let path = directory.path().join("说明.md");
        std::fs::write(&path, b"# Before\n").expect("write markdown fixture");
        let resource_id = Uuid::new_v4();
        let preview = resolve_local_preview(resource_id, &path).expect("resolve local preview");

        assert_eq!(preview.descriptor.source, PreviewSource::Local);
        assert_eq!(
            decode_preview_id(&preview.descriptor.id).expect("decode local preview id"),
            PreviewTarget::Local { resource_id }
        );
        assert!(preview.descriptor.capabilities.read);
        assert!(preview.descriptor.capabilities.write);
        assert!(preview.descriptor.capabilities.watch);

        write_preview_content(
            &preview,
            &preview.descriptor.revision,
            b"# After\n",
            MAX_PREVIEW_CONTENT_BYTES,
        )
        .expect("commit local edit");
        assert_eq!(std::fs::read(&path).unwrap(), b"# After\n");

        let current = resolve_local_preview(resource_id, &path).expect("refresh local preview");
        let error = write_preview_content(
            &current,
            &preview.descriptor.revision,
            b"stale",
            MAX_PREVIEW_CONTENT_BYTES,
        )
        .expect_err("stale revision must fail");
        assert!(matches!(error, PreviewError::RevisionConflict { .. }));
    }

    #[test]
    fn preview_write_rechecks_the_file_revision_at_commit_time() {
        let directory = TestDirectory::new();
        let path = directory.path().join("concurrent.md");
        std::fs::write(&path, b"initial").expect("write initial fixture");
        let preview = resolve_local_preview(Uuid::new_v4(), &path).unwrap();

        std::fs::write(&path, b"external concurrent change").expect("change fixture externally");
        let error = write_preview_content(
            &preview,
            &preview.descriptor.revision,
            b"stale replacement",
            MAX_PREVIEW_CONTENT_BYTES,
        )
        .expect_err("stale preview must not overwrite a newer file");

        assert!(matches!(error, PreviewError::RevisionConflict { .. }));
        assert_eq!(std::fs::read(&path).unwrap(), b"external concurrent change");
    }

    #[test]
    fn preview_writes_preserve_an_existing_utf8_bom() {
        let directory = TestDirectory::new();
        let path = directory.path().join("bom.md");
        std::fs::write(&path, b"\xef\xbb\xbf# Before\r\n").expect("write BOM fixture");
        let preview = resolve_local_preview(Uuid::new_v4(), &path).unwrap();
        write_preview_content(
            &preview,
            &preview.descriptor.revision,
            b"# After\r\n",
            MAX_PREVIEW_CONTENT_BYTES,
        )
        .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"\xef\xbb\xbf# After\r\n");
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
    fn attachment_preview_round_trips_opaque_id_outside_workspace() {
        let directory = TestDirectory::new();
        let path = directory.path().join("brief.pdf");
        std::fs::write(&path, b"%PDF-1.7").expect("write attachment");
        let attachment = ContextSourceRef {
            id: Uuid::new_v4(),
            path: path.clone(),
            name: "brief.pdf".to_string(),
            kind: ContextSourceKind::Document,
            content_type: "application/pdf".to_string(),
            bytes: 0,
            truncated: false,
        };

        let preview = resolve_attachment_preview(&attachment).expect("resolve attachment preview");
        assert_eq!(preview.descriptor.kind, PreviewKind::Pdf);
        assert_eq!(preview.descriptor.source, PreviewSource::Attachment);
        assert_eq!(
            preview.descriptor.path,
            Some(path.canonicalize().expect("canonical attachment path"))
        );
        assert_eq!(preview.descriptor.bytes, 8);
        assert_eq!(
            decode_preview_id(&preview.descriptor.id).expect("decode attachment preview id"),
            PreviewTarget::Attachment {
                attachment_id: attachment.id,
            }
        );
    }

    #[test]
    fn docx_attachment_uses_the_document_preview_pipeline() {
        let directory = TestDirectory::new();
        let path = directory.path().join("brief.docx");
        std::fs::write(&path, b"docx fixture").expect("write attachment");
        let attachment = ContextSourceRef {
            id: Uuid::new_v4(),
            path,
            name: "brief.docx".to_string(),
            kind: ContextSourceKind::Document,
            content_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                .to_string(),
            bytes: 0,
            truncated: false,
        };

        let preview = resolve_attachment_preview(&attachment).expect("resolve DOCX preview");
        assert_eq!(preview.descriptor.kind, PreviewKind::Document);
        assert_eq!(
            preview.descriptor.content_type,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
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
                    style_from: None,
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
    fn spreadsheet_preview_formats_excel_dates_without_losing_raw_values() {
        let directory = TestDirectory::new();
        let workbook_path = directory.path().join("dates.xlsx");
        let mut workbook = Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("Orders").expect("set sheet name");
        worksheet
            .write_number_with_format(
                0,
                0,
                46_252.932_719_907_41,
                &Format::new().set_num_format("yyyy-mm-dd"),
            )
            .expect("write date-only cell");
        worksheet
            .write_number_with_format(
                0,
                1,
                46_252.932_719_907_41,
                &Format::new().set_num_format("yyyy-mm-dd hh:mm:ss"),
            )
            .expect("write date-time cell");
        workbook.save(&workbook_path).expect("save workbook");

        let preview = resolve_workspace_preview(directory.path(), Path::new("dates.xlsx"))
            .expect("resolve workbook preview");
        let range = preview_spreadsheet_range(
            &preview,
            PreviewRangeRequest {
                sheet: "Orders".to_string(),
                start_row: 0,
                start_column: 0,
                row_count: 1,
                column_count: 2,
            },
        )
        .expect("read formatted workbook range");

        assert_eq!(range.rows[0][0].formatted.as_deref(), Some("2026-08-18"));
        assert_eq!(
            range.rows[0][1].formatted.as_deref(),
            Some("2026-08-18 22:23:07")
        );
        assert!(matches!(
            range.rows[0][0].value,
            SpreadsheetCellValue::DateTime(value)
                if (value.serial - 46_252.932_719_907_41).abs() < f64::EPSILON
        ));
    }

    #[test]
    fn legacy_xls_attachment_keeps_its_logical_extension_for_the_reader() {
        let directory = TestDirectory::new();
        let opaque_path = directory.path().join("upload.bin");
        std::fs::write(&opaque_path, b"not a real workbook").expect("write opaque attachment");
        let attachment = ContextSourceRef {
            id: Uuid::new_v4(),
            path: opaque_path,
            name: "legacy.xls".to_string(),
            kind: ContextSourceKind::Document,
            content_type: "application/vnd.ms-excel".to_string(),
            bytes: 0,
            truncated: false,
        };
        let preview =
            resolve_attachment_preview(&attachment).expect("resolve opaque XLS attachment");
        assert_eq!(preview.descriptor.kind, PreviewKind::Spreadsheet);
        let staged = spreadsheet_path(&preview).expect("stage opaque XLS attachment");
        assert_eq!(
            staged
                .as_path()
                .extension()
                .and_then(|value| value.to_str()),
            Some("xls")
        );
    }

    #[test]
    fn csv_preview_uses_virtualized_workbook_ranges() {
        let directory = TestDirectory::new();
        let csv_path = directory.path().join("orders.csv");
        std::fs::write(
            &csv_path,
            "order_id,customer,note\n1001,Alice,first line\n1002,Bob,\"contains, comma\"\n",
        )
        .expect("write CSV fixture");

        let preview = resolve_workspace_preview(directory.path(), Path::new("orders.csv"))
            .expect("resolve CSV preview");
        assert_eq!(preview.descriptor.kind, PreviewKind::Spreadsheet);

        let workbook = preview_workbook(&preview).expect("read CSV metadata");
        assert_eq!(workbook.sheets.len(), 1);
        assert_eq!(workbook.sheets[0].name, DELIMITED_PREVIEW_SHEET);
        assert_eq!(workbook.sheets[0].row_count, 3);
        assert_eq!(workbook.sheets[0].column_count, 3);

        let range = preview_spreadsheet_range(
            &preview,
            PreviewRangeRequest {
                sheet: DELIMITED_PREVIEW_SHEET.to_string(),
                start_row: 1,
                start_column: 1,
                row_count: 2,
                column_count: 2,
            },
        )
        .expect("read CSV range");
        assert_eq!(range.rows.len(), 2);
        assert_eq!(
            range.rows[0][0].value,
            SpreadsheetCellValue::String("Alice".to_string())
        );
        assert_eq!(
            range.rows[1][1].value,
            SpreadsheetCellValue::String("contains, comma".to_string())
        );
    }

    #[test]
    fn tsv_attachment_is_classified_by_its_name_when_stored_as_a_temp_file() {
        let directory = TestDirectory::new();
        let path = directory.path().join("upload.bin");
        std::fs::write(&path, "name\tvalue\nalpha\t1\n").expect("write TSV fixture");
        let attachment = ContextSourceRef {
            id: Uuid::new_v4(),
            path,
            name: "metrics.tsv".to_string(),
            kind: ContextSourceKind::Document,
            content_type: "text/plain; charset=utf-8".to_string(),
            bytes: 0,
            truncated: false,
        };

        let preview = resolve_attachment_preview(&attachment).expect("resolve TSV preview");
        assert_eq!(preview.descriptor.kind, PreviewKind::Spreadsheet);
        let workbook = preview_workbook(&preview).expect("read TSV metadata");
        assert_eq!(workbook.sheets[0].row_count, 2);
        assert_eq!(workbook.sheets[0].column_count, 2);
    }

    #[test]
    fn csv_attachment_is_classified_from_mime_with_charset_parameters() {
        let directory = TestDirectory::new();
        let path = directory.path().join("upload.bin");
        std::fs::write(&path, "name,value\nalpha,1\n").expect("write CSV fixture");
        let attachment = ContextSourceRef {
            id: Uuid::new_v4(),
            path,
            name: "orders.csv".to_string(),
            kind: ContextSourceKind::Document,
            content_type: "text/csv; charset=utf-8".to_string(),
            bytes: 0,
            truncated: false,
        };

        let preview = resolve_attachment_preview(&attachment).expect("resolve CSV preview");
        assert_eq!(preview.descriptor.kind, PreviewKind::Spreadsheet);
    }

    #[test]
    fn inline_csv_artifact_can_be_previewed_as_a_spreadsheet() {
        let directory = TestDirectory::new();
        let thread_id = Uuid::new_v4();
        let artifact = Artifact::inline(
            thread_id,
            "spreadsheet",
            "text/csv; charset=utf-8",
            "name,value\nalpha,1\n",
            json!({"name": "metrics.csv"}),
        );

        let preview = resolve_artifact_preview(thread_id, directory.path(), &artifact)
            .expect("resolve inline CSV artifact");
        assert_eq!(preview.descriptor.kind, PreviewKind::Spreadsheet);
        let workbook = preview_workbook(&preview).expect("read inline CSV metadata");
        assert_eq!(workbook.sheets[0].row_count, 2);
        assert_eq!(workbook.sheets[0].column_count, 2);
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
