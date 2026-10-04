// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use shared::AppError;

#[derive(Debug)]
pub struct HttpError {
    pub status: StatusCode,
    pub message: String,
}

impl HttpError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl From<AppError> for HttpError {
    fn from(err: AppError) -> Self {
        let (status, message) = match err {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::Validation(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            AppError::RateLimited(msg) => (StatusCode::TOO_MANY_REQUESTS, msg),
            AppError::Upstream(msg) => (StatusCode::BAD_GATEWAY, msg),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };
        Self { status, message }
    }
}

impl From<axum::extract::rejection::JsonRejection> for HttpError {
    fn from(rejection: axum::extract::rejection::JsonRejection) -> Self {
        let status = match rejection.status() {
            StatusCode::UNSUPPORTED_MEDIA_TYPE => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            StatusCode::PAYLOAD_TOO_LARGE => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::BAD_REQUEST,
        };
        Self {
            status,
            message: rejection.body_text(),
        }
    }
}

impl From<axum::extract::rejection::QueryRejection> for HttpError {
    fn from(rejection: axum::extract::rejection::QueryRejection) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: rejection.body_text(),
        }
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

/// Extractor JSON que unifica las respuestas de error en formato JSON {"error": ...}
/// y preserva los códigos de estado HTTP adecuados (400, 413, 415).
#[derive(Debug, Clone, Copy, Default)]
pub struct AppJson<T>(pub T);

impl<S, T> axum::extract::FromRequest<S> for AppJson<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = HttpError;

    async fn from_request(
        req: axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(val)) => Ok(AppJson(val)),
            Err(rejection) => Err(HttpError::from(rejection)),
        }
    }
}

impl<T> IntoResponse for AppJson<T>
where
    T: serde::Serialize,
{
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}
