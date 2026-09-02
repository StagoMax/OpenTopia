use crate::{
    contributions_api, ensure_thread, parse_resource_preview_id, resource_preview_id, ApiError,
    AppState, Artifact, ArtifactMetadata, MediaHandlerSelection, MessagePart, PreviewDescriptor,
    PreviewError, PreviewKind, PreviewRange, PreviewRangeRequest, PreviewWorkbook, ResolvedPreview,
    ResourceLease, ResourceLocator, SessionStore, SqliteSessionStore, MAX_PREVIEW_CONTENT_BYTES,
};
#[cfg(test)]
use crate::{plugin_runtime, ContributionKind, PreviewTarget};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/threads/:thread_id/artifacts", get(list_artifacts))
        .route(
            "/api/threads/:thread_id/artifacts/:artifact_id",
            get(get_artifact),
        )
        .route(
            "/api/threads/:thread_id/previews/resolve",
            post(resolve_preview),
        )
        .route(
            "/api/threads/:thread_id/previews/:preview_id/content",
            get(read_preview_content),
        )
        .route(
            "/api/threads/:thread_id/previews/:preview_id/workbook",
            get(get_preview_workbook),
        )
        .route(
            "/api/threads/:thread_id/previews/:preview_id/range",
            get(read_preview_range),
        )
        .route(
            "/api/threads/:thread_id/resources/resolve",
            post(resolve_preview),
        )
        .route(
            "/api/threads/:thread_id/resources/:preview_id",
            get(get_resource_metadata).delete(release_resource),
        )
        .route(
            "/api/threads/:thread_id/resources/:preview_id/content",
            get(read_preview_content)
                .put(write_resource_content)
                .layer(DefaultBodyLimit::max(MAX_PREVIEW_CONTENT_BYTES as usize)),
        )
        .route(
            "/api/threads/:thread_id/resources/:preview_id/workbook",
            get(get_preview_workbook),
        )
        .route(
            "/api/threads/:thread_id/resources/:preview_id/range",
            get(read_preview_range),
        )
}

async fn list_artifacts(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<ArtifactMetadata>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(state.store.list_artifacts(thread_id)?))
}

async fn get_artifact(
    State(state): State<AppState>,
    Path((thread_id, artifact_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Artifact>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let artifact = state
        .store
        .get_artifact(thread_id, artifact_id)?
        .ok_or_else(|| ApiError::not_found(format!("artifact not found: {artifact_id}")))?;
    Ok(Json(artifact))
}

async fn resolve_preview(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(target): Json<ResourceResolveRequest>,
) -> Result<Json<PreviewDescriptor>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let lease = state.resources.register(thread_id, target.into_locator());
    let preview = match resolve_resource_lease(&state.store, &thread, &lease) {
        Ok(preview) => preview,
        Err(error) => {
            state.resources.release(thread_id, lease.id);
            return Err(error);
        }
    };
    if let Some(normalized) = normalized_resource_locator(&lease.locator, &preview.descriptor) {
        state.resources.replace_locator(lease.id, normalized);
    }
    let mut descriptor = preview.descriptor;
    descriptor.id = resource_preview_id(lease.id);
    if descriptor.kind != PreviewKind::Unsupported {
        return Ok(Json(descriptor));
    }

    // Host-owned file renderers are part of the desktop application and do
    // not depend on model-tool plugin activation. Plugin previewers extend
    // formats the host does not understand; they do not gate built-in ones.
    let handlers = contributions_api::handler_registry_for_thread(&state, &thread)?;
    match handlers.select_previewer(descriptor.path.as_deref(), Some(&descriptor.content_type)) {
        MediaHandlerSelection::Selected { handler } => {
            descriptor.handler_id = Some(handler.contribution_id);
        }
        MediaHandlerSelection::Conflict { contribution_ids } => {
            return Err(ApiError::conflict(format!(
                "multiple preview handlers have equal priority: {}",
                contribution_ids.join(", ")
            )));
        }
        MediaHandlerSelection::None => {}
    }
    Ok(Json(descriptor))
}

async fn get_resource_metadata(
    State(state): State<AppState>,
    Path((thread_id, preview_id)): Path<(Uuid, String)>,
) -> Result<Json<PreviewDescriptor>, ApiError> {
    Ok(Json(
        resolve_preview_id_for_thread(&state, thread_id, &preview_id)?.descriptor,
    ))
}

async fn write_resource_content(
    State(state): State<AppState>,
    Path((thread_id, preview_id)): Path<(Uuid, String)>,
    Json(request): Json<ResourceWriteRequest>,
) -> Result<Json<PreviewDescriptor>, ApiError> {
    let preview = resolve_preview_id_for_thread(&state, thread_id, &preview_id)?;
    tokio::task::spawn_blocking(move || {
        opentopia_core::write_preview_content(
            &preview,
            &request.expected_revision,
            request.content.as_bytes(),
            MAX_PREVIEW_CONTENT_BYTES,
        )
    })
    .await
    .map_err(|error| ApiError::internal(format!("resource write worker failed: {error}")))?
    .map_err(preview_api_error)?;
    Ok(Json(
        resolve_preview_id_for_thread(&state, thread_id, &preview_id)?.descriptor,
    ))
}

async fn release_resource(
    State(state): State<AppState>,
    Path((thread_id, preview_id)): Path<(Uuid, String)>,
) -> Result<Json<ResourceReleaseResponse>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let resource_id = parse_resource_preview_id(&preview_id)
        .ok_or_else(|| ApiError::bad_request("invalid resource id"))?;
    Ok(Json(ResourceReleaseResponse {
        released: state.resources.release(thread_id, resource_id),
    }))
}

async fn read_preview_content(
    State(state): State<AppState>,
    Path((thread_id, preview_id)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let preview = resolve_preview_id_for_thread(&state, thread_id, &preview_id)?;
    let descriptor = preview.descriptor.clone();
    let document_preview = descriptor.kind == PreviewKind::Document;
    let etag = format!("\"{}\"", descriptor.revision);
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag))
    {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        response.headers_mut().insert(
            header::ETAG,
            HeaderValue::from_str(&etag).expect("preview revisions are valid header values"),
        );
        return Ok(response);
    }

    let mut bytes = tokio::task::spawn_blocking(move || {
        opentopia_core::read_preview_content(&preview, MAX_PREVIEW_CONTENT_BYTES)
    })
    .await
    .map_err(|error| ApiError::internal(format!("preview content worker failed: {error}")))?
    .map_err(preview_api_error)?;
    if document_preview {
        // Attachment and artifact storage paths may be opaque. Prefer a
        // declared DOCX filename, then a DOCX storage path, and finally a
        // synthetic logical name; the bytes still come from the thread-scoped
        // resolved preview.
        let declared_path = PathBuf::from(&descriptor.name);
        let validation_path = if declared_path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("docx"))
        {
            declared_path
        } else if descriptor.path.as_deref().is_some_and(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("docx"))
        }) {
            descriptor.path.clone().expect("DOCX path checked")
        } else {
            PathBuf::from("preview.docx")
        };
        bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, ApiError> {
            opentopia_core::inspect_document(&validation_path, &bytes).map_err(|error| {
                ApiError::bad_request(format!("DOCX preview is unavailable: {error}"))
            })?;
            Ok(bytes)
        })
        .await
        .map_err(|error| ApiError::internal(format!("DOCX preview worker failed: {error}")))??;
    }

    let content_length = bytes.len();
    let mut response = Response::new(Body::from(bytes));
    let content_type = HeaderValue::from_str(&descriptor.content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&content_length.to_string())
            .expect("content length is a valid header value"),
    );
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("preview revisions are valid header values"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "sandbox; default-src 'none'; img-src data: blob:; style-src 'unsafe-inline'",
        ),
    );
    Ok(response)
}

async fn get_preview_workbook(
    State(state): State<AppState>,
    Path((thread_id, preview_id)): Path<(Uuid, String)>,
) -> Result<Json<PreviewWorkbook>, ApiError> {
    let preview = resolve_preview_id_for_thread(&state, thread_id, &preview_id)?;
    let workbook = tokio::task::spawn_blocking(move || opentopia_core::preview_workbook(&preview))
        .await
        .map_err(|error| ApiError::internal(format!("workbook preview worker failed: {error}")))?
        .map_err(preview_api_error)?;
    Ok(Json(workbook))
}

async fn read_preview_range(
    State(state): State<AppState>,
    Path((thread_id, preview_id)): Path<(Uuid, String)>,
    Query(query): Query<PreviewRangeQuery>,
) -> Result<Json<PreviewRange>, ApiError> {
    let preview = resolve_preview_id_for_thread(&state, thread_id, &preview_id)?;
    let request = PreviewRangeRequest {
        sheet: query.sheet,
        start_row: query.start_row.unwrap_or(0),
        start_column: query.start_column.unwrap_or(0),
        row_count: query.row_count.unwrap_or(100),
        column_count: query.column_count.unwrap_or(26),
    };
    let range = tokio::task::spawn_blocking(move || {
        opentopia_core::preview_spreadsheet_range(&preview, request)
    })
    .await
    .map_err(|error| ApiError::internal(format!("spreadsheet preview worker failed: {error}")))?
    .map_err(preview_api_error)?;
    Ok(Json(range))
}

#[cfg(test)]
pub(super) fn bundled_plugin_enabled_for_thread(
    store: &SqliteSessionStore,
    thread_id: Uuid,
    plugin_name: &str,
) -> Result<bool, ApiError> {
    let thread = store
        .get_thread(thread_id)?
        .ok_or_else(|| ApiError::not_found(format!("thread not found: {thread_id}")))?;
    let outcome = plugin_runtime::load_plugin_outcome_for_thread(store, &thread)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let Some(plugin) = outcome
        .descriptors()
        .find(|plugin| plugin.name == plugin_name && !plugin.native_capabilities.is_empty())
    else {
        return Ok(false);
    };
    let has_native_tool = outcome.active_contributions().any(|contribution| {
        contribution.plugin_id == plugin.id && contribution.kind == ContributionKind::NativeTool
    });
    Ok(has_native_tool)
}

pub(crate) fn resolve_preview_id_for_thread(
    state: &AppState,
    thread_id: Uuid,
    preview_id: &str,
) -> Result<ResolvedPreview, ApiError> {
    let thread = ensure_thread(state, thread_id)?;
    let resource_id = parse_resource_preview_id(preview_id)
        .ok_or_else(|| ApiError::bad_request("invalid resource id"))?;
    let lease = state
        .resources
        .get(thread_id, resource_id)
        .ok_or_else(|| ApiError::not_found("resource handle was not found or has expired"))?;
    let mut preview = resolve_resource_lease(&state.store, &thread, &lease)?;
    preview.descriptor.id = preview_id.to_string();
    Ok(preview)
}

fn resolve_resource_lease(
    store: &SqliteSessionStore,
    thread: &opentopia_core::Thread,
    lease: &ResourceLease,
) -> Result<ResolvedPreview, ApiError> {
    match &lease.locator {
        ResourceLocator::Workspace { path } => {
            opentopia_core::resolve_workspace_preview(&thread.workspace_root, path)
                .map_err(preview_api_error)
        }
        ResourceLocator::Local { path } => {
            opentopia_core::resolve_local_preview(lease.id, path).map_err(preview_api_error)
        }
        ResourceLocator::Artifact { artifact_id } => {
            let artifact = store
                .get_artifact(thread.id, *artifact_id)?
                .ok_or_else(|| ApiError::not_found(format!("artifact not found: {artifact_id}")))?;
            opentopia_core::resolve_artifact_preview(thread.id, &thread.workspace_root, &artifact)
                .map_err(preview_api_error)
        }
        ResourceLocator::Attachment { attachment_id } => {
            let attachment = store
                .list_messages(thread.id)?
                .into_iter()
                .flat_map(|message| message.parts)
                .find_map(|part| match part {
                    MessagePart::SourceRef { source, .. } if source.id == *attachment_id => {
                        Some(source)
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    ApiError::not_found(format!("attachment not found: {attachment_id}"))
                })?;
            opentopia_core::resolve_attachment_preview(&attachment).map_err(preview_api_error)
        }
    }
}

fn normalized_resource_locator(
    original: &ResourceLocator,
    descriptor: &PreviewDescriptor,
) -> Option<ResourceLocator> {
    match original {
        ResourceLocator::Workspace { .. } => descriptor
            .path
            .clone()
            .map(|path| ResourceLocator::Workspace { path }),
        ResourceLocator::Local { .. } => descriptor
            .path
            .clone()
            .map(|path| ResourceLocator::Local { path }),
        ResourceLocator::Artifact { artifact_id } => Some(ResourceLocator::Artifact {
            artifact_id: *artifact_id,
        }),
        ResourceLocator::Attachment { attachment_id } => Some(ResourceLocator::Attachment {
            attachment_id: *attachment_id,
        }),
    }
}

#[cfg(test)]
pub(super) fn resolve_preview_target(
    store: &SqliteSessionStore,
    thread: &opentopia_core::Thread,
    target: &PreviewTarget,
) -> Result<ResolvedPreview, ApiError> {
    match target {
        PreviewTarget::Workspace { path } => {
            opentopia_core::resolve_workspace_preview(&thread.workspace_root, path)
                .map_err(preview_api_error)
        }
        PreviewTarget::Local { .. } => Err(ApiError::bad_request(
            "local resources must be resolved through the resource registry",
        )),
        PreviewTarget::Artifact { artifact_id } => {
            let artifact = store
                .get_artifact(thread.id, *artifact_id)?
                .ok_or_else(|| ApiError::not_found(format!("artifact not found: {artifact_id}")))?;
            opentopia_core::resolve_artifact_preview(thread.id, &thread.workspace_root, &artifact)
                .map_err(preview_api_error)
        }
        PreviewTarget::Attachment { attachment_id } => {
            let attachment = store
                .list_messages(thread.id)?
                .into_iter()
                .flat_map(|message| message.parts)
                .find_map(|part| match part {
                    MessagePart::SourceRef { source, .. } if source.id == *attachment_id => {
                        Some(source)
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    ApiError::not_found(format!("attachment not found: {attachment_id}"))
                })?;
            opentopia_core::resolve_attachment_preview(&attachment).map_err(preview_api_error)
        }
    }
}

pub(crate) fn preview_api_error(error: PreviewError) -> ApiError {
    let status = match &error {
        PreviewError::WorkspaceRootNotFound(_) | PreviewError::PathNotFound(_) => {
            StatusCode::NOT_FOUND
        }
        PreviewError::ArtifactThreadMismatch { .. } => StatusCode::NOT_FOUND,
        PreviewError::ContentTooLarge { .. }
        | PreviewError::Spreadsheet(opentopia_core::SpreadsheetError::FileTooLarge { .. }) => {
            StatusCode::PAYLOAD_TOO_LARGE
        }
        PreviewError::NotSpreadsheet(_) | PreviewError::InlineSpreadsheetUnsupported => {
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        }
        PreviewError::Spreadsheet(opentopia_core::SpreadsheetError::SheetNotFound { .. }) => {
            StatusCode::NOT_FOUND
        }
        PreviewError::Io { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        PreviewError::Delimited { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        PreviewError::RevisionConflict { .. } => StatusCode::CONFLICT,
        PreviewError::ReadOnly(_) => StatusCode::FORBIDDEN,
        PreviewError::InvalidPreviewId(_)
        | PreviewError::ParentDirectoryNotAllowed
        | PreviewError::OutsideWorkspace(_)
        | PreviewError::NotAFile(_)
        | PreviewError::InvalidRange(_)
        | PreviewError::Spreadsheet(_) => StatusCode::BAD_REQUEST,
    };
    ApiError {
        status,
        message: error.to_string(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewRangeQuery {
    sheet: String,
    start_row: Option<u32>,
    start_column: Option<u32>,
    row_count: Option<u32>,
    column_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "source",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum ResourceResolveRequest {
    Workspace { path: PathBuf },
    Local { path: PathBuf },
    Artifact { artifact_id: Uuid },
    Attachment { attachment_id: Uuid },
}

impl ResourceResolveRequest {
    pub(super) fn into_locator(self) -> ResourceLocator {
        match self {
            Self::Workspace { path } => ResourceLocator::Workspace { path },
            Self::Local { path } => ResourceLocator::Local { path },
            Self::Artifact { artifact_id } => ResourceLocator::Artifact { artifact_id },
            Self::Attachment { attachment_id } => ResourceLocator::Attachment { attachment_id },
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResourceWriteRequest {
    expected_revision: String,
    content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceReleaseResponse {
    pub(crate) released: bool,
}
