use axum::{
    http::{header::RETRY_AFTER, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use serde_json::Value;

pub type AppResult<T> = Result<T, AppError>;

const RETRY_AFTER_SECS: &str = "60";

#[derive(Debug, Serialize)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),

    #[error("{0}")]
    Unauthorized(String),

    #[error("{0}")]
    Forbidden(String),

    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    Conflict(String),

    #[error("{0}")]
    TooManyRequests(String),

    #[error("{0}")]
    Upstream(String),

    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("upstream transport error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) | Self::Json(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Upstream(_) => StatusCode::BAD_GATEWAY,
            Self::Database(_) | Self::Sqlx(_) | Self::Io(_) | Self::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Http(err) => {
                if err.is_timeout() {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                }
            }
        }
    }

    pub fn error_type(&self) -> &'static str {
        match self {
            Self::BadRequest(_) | Self::Json(_) => "invalid_request_error",
            Self::Unauthorized(_) => "authentication_error",
            Self::Forbidden(_) => "permission_error",
            Self::NotFound(_) => "not_found_error",
            Self::Conflict(_) => "conflict_error",
            Self::TooManyRequests(_) => "rate_limit_error",
            Self::Upstream(_) => "upstream_error",
            _ => "server_error",
        }
    }

    pub fn error_code(&self) -> Option<&'static str> {
        match self {
            Self::BadRequest(_) | Self::Json(_) => Some("invalid_request"),
            Self::Unauthorized(_) => Some("invalid_api_key"),
            Self::Forbidden(_) => Some("forbidden"),
            Self::NotFound(_) => Some("not_found"),
            Self::Conflict(_) => Some("conflict"),
            Self::TooManyRequests(_) => Some("rate_limit_exceeded"),
            Self::Upstream(_) => Some("upstream_error"),
            Self::Database(_) | Self::Sqlx(_) => Some("database_error"),
            Self::Http(err) if err.is_timeout() => Some("upstream_timeout"),
            Self::Http(_) => Some("upstream_transport_error"),
            Self::Io(_) | Self::Internal(_) => Some("internal_error"),
        }
    }

    pub fn envelope(&self) -> Value {
        serde_json::to_value(ErrorEnvelope {
            error: ErrorBody {
                message: self.to_string(),
                error_type: self.error_type().to_string(),
                code: self.error_code().map(str::to_string),
                param: None,
            },
        })
        .unwrap_or_else(|_| {
            serde_json::json!({
                "error": {
                    "message": "internal error",
                    "type": "server_error",
                    "code": "internal_error"
                }
            })
        })
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        if status.is_server_error() {
            tracing::error!(error = %self, status = status.as_u16(), "request failed");
        } else {
            tracing::debug!(error = %self, status = status.as_u16(), "request rejected");
        }

        let mut response = (status, Json(self.envelope())).into_response();
        if status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from_static(RETRY_AFTER_SECS));
        }
        response
    }
}
