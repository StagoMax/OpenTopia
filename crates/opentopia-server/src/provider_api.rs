//! Provider configuration, account lifecycle, and model catalog HTTP domain.

mod models;
mod settings;

use crate::AppState;
use axum::Router;

pub(crate) use models::ProviderModelSyncResult;
#[cfg(test)]
pub(super) use models::{
    extract_model_catalog, provider_model_catalog_rate_limit_delay, provider_model_catalog_url,
};
#[cfg(test)]
pub(super) use settings::validate_provider_settings;
pub(super) use settings::{current_settings, save_settings_and_refresh_runtime};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .merge(settings::router())
        .merge(models::router())
}
