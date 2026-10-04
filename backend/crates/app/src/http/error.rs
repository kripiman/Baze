// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use shared::AppError;
use utoipa::ToSchema;
use uuid::Uuid;

/// Body of every error response.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Human-readable reason. For 5xx responses it is deliberately generic.
    pub error: String,
    /// Present on 5xx responses: quote it when reporting a problem, the details are in the server log under this id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_id: Option<Uuid>,
}

#[derive(Debug)]
pub struct HttpError {
    pub status: StatusCode,
    pub message: String,
    pub error_id: Option<Uuid>,
}

impl HttpError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            error_id: None,
        }
    }

    /// A server-side failure: the client gets a generic message and a correlation id, the detail
    /// (which may name hosts, SQL or paths) goes only to the log.
    fn server_error(status: StatusCode, public_message: &str, detail: &str) -> Self {
        let error_id = Uuid::new_v4();
        tracing::error!(%error_id, %status, detail, "request failed");
        Self {
            status,
            message: public_message.to_string(),
            error_id: Some(error_id),
        }
    }
}

impl From<AppError> for HttpError {
    fn from(err: AppError) -> Self {
        match err {
            AppError::NotFound(msg) => Self::new(StatusCode::NOT_FOUND, msg),
            AppError::Unauthorized(msg) => Self::new(StatusCode::UNAUTHORIZED, msg),
            AppError::Validation(msg) => Self::new(StatusCode::BAD_REQUEST, msg),
            AppError::Conflict(msg) => Self::new(StatusCode::CONFLICT, msg),
            AppError::RateLimited(msg) => Self::new(StatusCode::TOO_MANY_REQUESTS, msg),
            AppError::NotImplemented(msg) => Self::new(StatusCode::NOT_IMPLEMENTED, msg),
            AppError::Upstream(detail) => Self::server_error(
                StatusCode::BAD_GATEWAY,
                "A backing service did not respond correctly",
                &detail,
            ),
            AppError::Internal(detail) => Self::server_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
                &detail,
            ),
        }
    }
}

impl From<axum::extract::rejection::JsonRejection> for HttpError {
    fn from(rejection: axum::extract::rejection::JsonRejection) -> Self {
        let status = match rejection.status() {
            StatusCode::UNSUPPORTED_MEDIA_TYPE => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            StatusCode::PAYLOAD_TOO_LARGE => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::BAD_REQUEST,
        };
        Self::new(status, rejection.body_text())
    }
}

impl From<axum::extract::rejection::QueryRejection> for HttpError {
    fn from(rejection: axum::extract::rejection::QueryRejection) -> Self {
        Self::new(StatusCode::BAD_REQUEST, rejection.body_text())
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
                error_id: self.error_id,
            }),
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

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    async fn body_json(response: Response) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn internal_errors_do_not_leak_details_and_carry_an_error_id() {
        let secret_detail =
            "connection to postgres://baze_app:hunter2@db:5432 refused; Lock poisoned";
        let response = HttpError::from(AppError::Internal(secret_detail.into())).into_response();
        let (status, json) = body_json(response).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json["error"], "Internal server error");
        assert!(Uuid::parse_str(json["error_id"].as_str().unwrap()).is_ok());
        let text = json.to_string();
        assert!(
            !text.contains("hunter2") && !text.contains("postgres") && !text.contains("poisoned")
        );
    }

    #[tokio::test]
    async fn upstream_errors_are_generic_bad_gateway() {
        let response =
            HttpError::from(AppError::Upstream("valhalla:8002 timed out".into())).into_response();
        let (status, json) = body_json(response).await;

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(!json.to_string().contains("valhalla"));
        assert!(json["error_id"].is_string());
    }

    #[tokio::test]
    async fn client_errors_keep_their_message_and_have_no_error_id() {
        let cases = [
            (AppError::NotFound("nope".into()), StatusCode::NOT_FOUND),
            (
                AppError::Unauthorized("who?".into()),
                StatusCode::UNAUTHORIZED,
            ),
            (AppError::Validation("bad".into()), StatusCode::BAD_REQUEST),
            (AppError::Conflict("dup".into()), StatusCode::CONFLICT),
            (
                AppError::RateLimited("slow down".into()),
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                AppError::NotImplemented("later".into()),
                StatusCode::NOT_IMPLEMENTED,
            ),
        ];
        for (err, expected) in cases {
            let (status, json) = body_json(HttpError::from(err).into_response()).await;
            assert_eq!(status, expected);
            assert!(json.get("error_id").is_none(), "{json}");
            assert!(json["error"].is_string());
        }
    }
}
