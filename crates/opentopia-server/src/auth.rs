use crate::AppState;
use anyhow::{bail, Context};
use axum::extract::{Request, State};
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, ORIGIN, WWW_AUTHENTICATE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::env;
use std::time::Duration;
use tower_http::cors::{AllowOrigin, CorsLayer};

const API_TOKEN_ENV: &str = "OPENTOPIA_API_TOKEN";
const DEV_ORIGIN_ENV: &str = "OPENTOPIA_DEV_ORIGIN";
const MINIMUM_TOKEN_BYTES: usize = 32;
pub(crate) const TURN_ID_HEADER: HeaderName = HeaderName::from_static("x-opentopia-turn-id");

#[derive(Clone)]
pub(crate) struct ApiAuth {
    token: String,
    allowed_origins: Vec<String>,
}

impl ApiAuth {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let token = env::var(API_TOKEN_ENV).with_context(|| {
            format!("{API_TOKEN_ENV} is required; refusing to start without API authentication")
        })?;
        if token.as_bytes().len() < MINIMUM_TOKEN_BYTES {
            bail!("{API_TOKEN_ENV} must contain at least {MINIMUM_TOKEN_BYTES} bytes");
        }

        let mut allowed_origins = vec![
            "null".to_string(),
            "file:".to_string(),
            "file://".to_string(),
            "http://127.0.0.1:5173".to_string(),
            "http://localhost:5173".to_string(),
        ];
        if let Ok(origin) = env::var(DEV_ORIGIN_ENV) {
            let origin = origin.trim().trim_end_matches('/');
            if !origin.is_empty() {
                if !is_loopback_dev_origin(origin) {
                    bail!("{DEV_ORIGIN_ENV} must be an http(s) loopback origin without a path");
                }
                if !allowed_origins.iter().any(|allowed| allowed == origin) {
                    allowed_origins.push(origin.to_string());
                }
            }
        }

        Ok(Self {
            token,
            allowed_origins,
        })
    }

    pub(crate) fn cors_layer(&self) -> CorsLayer {
        let auth = self.clone();
        CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(move |origin, _parts| {
                auth.origin_allowed(Some(origin))
            }))
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PATCH,
                Method::PUT,
                Method::DELETE,
            ])
            .allow_headers([AUTHORIZATION, CONTENT_TYPE, ACCEPT])
            .expose_headers([TURN_ID_HEADER])
            .max_age(Duration::from_secs(600))
    }

    fn request_allowed(&self, headers: &HeaderMap) -> Result<(), AuthRejection> {
        if !self.origin_allowed(headers.get(ORIGIN)) {
            return Err(AuthRejection::ForbiddenOrigin);
        }

        let Some(value) = headers.get(AUTHORIZATION) else {
            return Err(AuthRejection::Unauthorized);
        };
        let Ok(value) = value.to_str() else {
            return Err(AuthRejection::Unauthorized);
        };
        let Some(candidate) = value.strip_prefix("Bearer ") else {
            return Err(AuthRejection::Unauthorized);
        };
        if candidate.is_empty() || !constant_time_eq(candidate.as_bytes(), self.token.as_bytes()) {
            return Err(AuthRejection::Unauthorized);
        }
        Ok(())
    }

    fn origin_allowed(&self, origin: Option<&HeaderValue>) -> bool {
        let Some(origin) = origin else {
            // Native clients do not send Origin. Browser requests are constrained below.
            return true;
        };
        let Ok(origin) = origin.to_str() else {
            return false;
        };
        if matches!(origin, "file:" | "file://") {
            return self.allowed_origins.iter().any(|allowed| allowed == origin);
        }
        let normalized = origin.trim_end_matches('/');
        self.allowed_origins
            .iter()
            .any(|allowed| allowed == normalized)
    }
}

pub(crate) async fn authorize(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    match state.auth.request_allowed(request.headers()) {
        Ok(()) => next.run(request).await,
        Err(rejection) => rejection.into_response(),
    }
}

enum AuthRejection {
    Unauthorized,
    ForbiddenOrigin,
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized => {
                let mut response = (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "unauthorized" })),
                )
                    .into_response();
                response.headers_mut().insert(
                    WWW_AUTHENTICATE,
                    HeaderValue::from_static("Bearer realm=\"opentopia-local-api\""),
                );
                response
            }
            Self::ForbiddenOrigin => (
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "origin not allowed" })),
            )
                .into_response(),
        }
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        difference |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

fn is_loopback_dev_origin(origin: &str) -> bool {
    let Ok(uri) = origin.parse::<axum::http::Uri>() else {
        return false;
    };
    if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.path() != "/" {
        return false;
    }
    matches!(uri.host(), Some("127.0.0.1" | "localhost" | "::1")) && uri.port_u16().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> ApiAuth {
        ApiAuth {
            token: "0123456789abcdef0123456789abcdef".to_string(),
            allowed_origins: vec!["null".to_string(), "http://127.0.0.1:5173".to_string()],
        }
    }

    #[test]
    fn accepts_valid_bearer_from_allowed_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(ORIGIN, HeaderValue::from_static("null"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer 0123456789abcdef0123456789abcdef"),
        );
        assert!(auth().request_allowed(&headers).is_ok());
    }

    #[test]
    fn rejects_missing_or_incorrect_bearer() {
        let headers = HeaderMap::new();
        assert!(matches!(
            auth().request_allowed(&headers),
            Err(AuthRejection::Unauthorized)
        ));

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer 0123456789abcdef0123456789abcdeg"),
        );
        assert!(matches!(
            auth().request_allowed(&headers),
            Err(AuthRejection::Unauthorized)
        ));
    }

    #[test]
    fn rejects_non_local_browser_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(ORIGIN, HeaderValue::from_static("https://attacker.example"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer 0123456789abcdef0123456789abcdef"),
        );
        assert!(matches!(
            auth().request_allowed(&headers),
            Err(AuthRejection::ForbiddenOrigin)
        ));
    }

    #[test]
    fn only_accepts_loopback_dev_origin_configuration() {
        assert!(is_loopback_dev_origin("http://127.0.0.1:5174"));
        assert!(is_loopback_dev_origin("http://localhost:4173"));
        assert!(!is_loopback_dev_origin("https://example.com:5173"));
        assert!(!is_loopback_dev_origin("http://127.0.0.1:5173/path"));
    }
}
