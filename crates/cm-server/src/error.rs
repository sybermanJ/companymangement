use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use cm_store::StoreError;
use serde_json::json;

/// Error returned to clients: a stable `code` for the UI and a readable message.
/// Internal details are logged, not sent.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_input", msg)
    }
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "sign in required")
    }
    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", "not allowed")
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::NotFound => Self::new(StatusCode::NOT_FOUND, "not_found", "not found"),
            StoreError::Invalid(why) => Self::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid", why),
            StoreError::Conflict(what) => Self::new(
                StatusCode::CONFLICT,
                "conflict",
                format!("{what} already exists"),
            ),
            other => {
                tracing::error!(error = %other, "store error");
                if std::env::var_os("CM_DEBUG_ERRORS").is_some() {
                    eprintln!("store error: {other}");
                }
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error",
                )
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "code": self.code, "message": self.message })),
        )
            .into_response()
    }
}
