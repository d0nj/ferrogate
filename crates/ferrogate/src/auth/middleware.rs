use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, HeaderMap, HeaderName},
    middleware::Next,
    response::Response,
};

use crate::{
    error::{AppError, AppResult},
    AppState,
};

const MODELS_PATH: &str = "/v1/models";

const X_API_KEY: HeaderName = HeaderName::from_static("x-api-key");
const X_GOOG_API_KEY: HeaderName = HeaderName::from_static("x-goog-api-key");

pub async fn require_virtual_key(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = credential(request.headers())?;
    let key = state.auth.authenticate(token).await?;

    if request.uri().path() != MODELS_PATH {
        state.auth.check_budget(&key)?;
        state.auth.admit_rpm(&key).await?;
    }

    request.extensions_mut().insert(key);
    Ok(next.run(request).await)
}

pub fn credential(headers: &HeaderMap) -> AppResult<&str> {
    if let Some(raw) = header_str(headers, AUTHORIZATION) {
        return bearer_token(raw);
    }

    for name in [X_API_KEY, X_GOOG_API_KEY] {
        let token = header_str(headers, name)
            .map(str::trim)
            .filter(|t| !t.is_empty());
        if let Some(token) = token {
            return Ok(token);
        }
    }

    Err(AppError::Unauthorized(
        "missing Authorization header".to_string(),
    ))
}

fn header_str(headers: &HeaderMap, name: HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn bearer_token(raw: &str) -> AppResult<&str> {
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            AppError::Unauthorized(
                "Authorization header must use the 'Bearer <key>' scheme".to_string(),
            )
        })
}
