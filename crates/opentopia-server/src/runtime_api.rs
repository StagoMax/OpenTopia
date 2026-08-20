use crate::AppState;
use axum::routing::post;
use axum::{Json, Router};
use opentopia_core::{
    retry_managed_office_runtime_install, retry_managed_powershell_install, OfficeRuntimeStatus,
    ShellRuntimeStatus,
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/runtime/powershell/retry",
            post(retry_managed_powershell),
        )
        .route(
            "/api/runtime/office/retry",
            post(retry_managed_office_runtime),
        )
}

async fn retry_managed_powershell() -> Json<ShellRuntimeStatus> {
    Json(retry_managed_powershell_install())
}

async fn retry_managed_office_runtime() -> Json<OfficeRuntimeStatus> {
    Json(retry_managed_office_runtime_install())
}
