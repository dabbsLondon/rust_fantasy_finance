use axum::{response::{IntoResponse, Response}, http::StatusCode, Json};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Serialize)]
struct ErrorMessage {
    error: String,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct AppError {
    pub status: StatusCode,
    pub message: String,
}

impl AppError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = Json(ErrorMessage { error: self.message });
        (self.status, body).into_response()
    }
}

impl From<crate::holdings::StoreError> for AppError {
    fn from(err: crate::holdings::StoreError) -> Self {
        match err {
            crate::holdings::StoreError::NoOrders(user) => {
                AppError::not_found(format!("no orders for user {user}"))
            }
            crate::holdings::StoreError::Other(e) => AppError::internal(e.to_string()),
        }
    }
}
