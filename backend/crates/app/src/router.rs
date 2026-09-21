// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::config::AppConfig;
use crate::openapi::{HealthResponse, SourceResponse};
use crate::rate_limit::RateLimiter;
use auth::{AuthResponse, AuthService};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures_util::stream::Stream;
use hazards::HazardService;
use realtime::RealtimeService;
use routing::ValhallaRoutingService;
use shared::{
    AppError, CreateHazardRequest, GeoJsonBbox, GeocodingItem, GeocodingProvider, Hazard,
    HazardNotifier, HazardReader, HazardVoteRequest, HazardWriter, RouteRequest, RouteResponse,
    RoutingProvider,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;
use uuid::Uuid;

pub struct AppState {
    pub config: AppConfig,
    pub auth_service: AuthService,
    pub hazard_service: HazardService,
    pub routing_service: ValhallaRoutingService,
    pub geocoding_service: Arc<dyn GeocodingProvider>,
    pub realtime_service: RealtimeService,
    pub rate_limiter: RateLimiter,
}

#[derive(Debug)]
pub struct HttpError(pub AppError);

impl From<AppError> for HttpError {
    fn from(err: AppError) -> Self {
        Self(err)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let (status, message) = match self.0 {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg),
            AppError::Validation(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            AppError::RateLimited(msg) => (StatusCode::TOO_MANY_REQUESTS, msg),
            AppError::Upstream(msg) => (StatusCode::BAD_GATEWAY, msg),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };
        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

fn check_rate_limit(state: &AppState, headers: &HeaderMap) -> Result<(), HttpError> {
    let client_key = if let Some(auth) = headers.get(header::AUTHORIZATION).and_then(|h| h.to_str().ok()) {
        format!("token:{}", auth)
    } else if let Some(xff) = headers.get("x-forwarded-for").and_then(|h| h.to_str().ok()) {
        format!("ip:{}", xff.split(',').next().unwrap_or(xff).trim())
    } else if let Some(xri) = headers.get("x-real-ip").and_then(|h| h.to_str().ok()) {
        format!("ip:{}", xri.trim())
    } else {
        "ip:unknown".to_string()
    };

    if state.rate_limiter.check(&client_key).is_err() {
        return Err(HttpError(AppError::RateLimited(
            "Rate limit exceeded. Please retry later.".into(),
        )));
    }
    Ok(())
}

fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, AppError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .ok_or_else(|| AppError::Unauthorized("Missing Authorization header".into()))?
        .to_str()
        .map_err(|_| AppError::Unauthorized("Invalid Authorization header encoding".into()))?;

    auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| AppError::Unauthorized("Authorization scheme must be Bearer".into()))
}

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", crate::openapi::ApiDoc::openapi()))
        .route("/health", get(health_handler))
        .route("/source", get(source_handler))
        .route("/api/v1/auth/anonymous", post(anonymous_auth_handler))
        .route("/api/v1/hazards", get(list_hazards_handler).post(create_hazard_handler))
        .route("/api/v1/hazards/{id}/vote", post(vote_hazard_handler))
        .route("/api/v1/routing/route", post(route_handler))
        .route("/api/v1/geocoding/search", get(geocoding_handler))
        .route("/api/v1/realtime/sse", get(realtime_sse_handler))
        .with_state(state)
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    responses(
        (status = 200, description = "Service is healthy", body = HealthResponse)
    )
)]
pub async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[utoipa::path(
    get,
    path = "/source",
    tag = "system",
    responses(
        (status = 200, description = "Source code repository and commit info (AGPL-3.0 compliance)", body = SourceResponse)
    )
)]
pub async fn source_handler(State(state): State<Arc<AppState>>) -> Json<SourceResponse> {
    Json(SourceResponse {
        repository: state.config.source_repo_url.clone(),
        commit: state.config.git_commit_hash.clone(),
        license: "AGPL-3.0-or-later".to_string(),
    })
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/anonymous",
    tag = "auth",
    responses(
        (status = 201, description = "Anonymous account created successfully", body = AuthResponse),
        (status = 429, description = "Rate limit exceeded"),
        (status = 500, description = "Internal error")
    )
)]
pub async fn anonymous_auth_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<AuthResponse>), HttpError> {
    check_rate_limit(&state, &headers)?;
    let response = state.auth_service.create_anonymous_account().await?;
    Ok((StatusCode::CREATED, Json(response)))
}

#[derive(serde::Deserialize, utoipa::IntoParams)]
pub struct BboxQuery {
    pub min_lon: f64,
    pub min_lat: f64,
    pub max_lon: f64,
    pub max_lat: f64,
}

#[utoipa::path(
    get,
    path = "/api/v1/hazards",
    tag = "hazards",
    params(BboxQuery),
    responses(
        (status = 200, description = "Active hazards within bounding box", body = Vec<Hazard>),
        (status = 400, description = "Invalid bbox query")
    )
)]
pub async fn list_hazards_handler(
    State(state): State<Arc<AppState>>,
    Query(bbox): Query<BboxQuery>,
) -> Result<Json<Vec<Hazard>>, HttpError> {
    let geo_bbox = GeoJsonBbox {
        min_lon: bbox.min_lon,
        min_lat: bbox.min_lat,
        max_lon: bbox.max_lon,
        max_lat: bbox.max_lat,
    };
    geo_bbox.validate()?;
    let hazards = state.hazard_service.list_active_hazards(&geo_bbox).await?;
    Ok(Json(hazards))
}

#[utoipa::path(
    post,
    path = "/api/v1/hazards",
    tag = "hazards",
    request_body = CreateHazardRequest,
    responses(
        (status = 201, description = "Hazard report created", body = Hazard),
        (status = 400, description = "Validation failed"),
        (status = 401, description = "Unauthorized caller"),
        (status = 429, description = "Rate limit exceeded")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn create_hazard_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateHazardRequest>,
) -> Result<(StatusCode, Json<Hazard>), HttpError> {
    check_rate_limit(&state, &headers)?;
    payload.validate()?;
    let token = extract_bearer_token(&headers)?;
    let account_id = state.auth_service.validate_token(token).await?;

    let hazard = state.hazard_service.create_hazard(account_id, &payload).await?;
    let _ = state.realtime_service.broadcast_hazard(&hazard).await;
    Ok((StatusCode::CREATED, Json(hazard)))
}

#[utoipa::path(
    post,
    path = "/api/v1/hazards/{id}/vote",
    tag = "hazards",
    params(
        ("id" = Uuid, Path, description = "Hazard ID")
    ),
    request_body = HazardVoteRequest,
    responses(
        (status = 200, description = "Vote registered and hazard updated", body = Hazard),
        (status = 400, description = "Invalid vote payload"),
        (status = 401, description = "Unauthorized caller"),
        (status = 404, description = "Hazard not found"),
        (status = 429, description = "Rate limit exceeded")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub async fn vote_hazard_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(payload): Json<HazardVoteRequest>,
) -> Result<Json<Hazard>, HttpError> {
    check_rate_limit(&state, &headers)?;
    payload.validate()?;
    let token = extract_bearer_token(&headers)?;
    let account_id = state.auth_service.validate_token(token).await?;

    let updated = state.hazard_service.vote_hazard(id, account_id, payload.vote_type).await?;
    let _ = state.realtime_service.broadcast_hazard(&updated).await;
    Ok(Json(updated))
}

#[utoipa::path(
    post,
    path = "/api/v1/routing/route",
    tag = "routing",
    request_body = RouteRequest,
    responses(
        (status = 200, description = "Calculated bicycle route", body = RouteResponse),
        (status = 400, description = "Invalid coordinates")
    )
)]
pub async fn route_handler(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RouteRequest>,
) -> Result<Json<RouteResponse>, HttpError> {
    payload.validate()?;
    let response = state.routing_service.route_bicycle(&payload).await?;
    Ok(Json(response))
}

#[derive(serde::Deserialize, utoipa::IntoParams)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<usize>,
}

#[utoipa::path(
    get,
    path = "/api/v1/geocoding/search",
    tag = "geocoding",
    params(SearchQuery),
    responses(
        (status = 200, description = "Geocoding suggestions", body = Vec<GeocodingItem>),
        (status = 400, description = "Invalid query parameters")
    )
)]
pub async fn geocoding_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<GeocodingItem>>, HttpError> {
    let query_str = query.q.trim();
    if query_str.is_empty() {
        return Err(HttpError(AppError::Validation("Query 'q' cannot be empty".into())));
    }
    if query_str.len() > 200 {
        return Err(HttpError(AppError::Validation(
            "Query 'q' exceeds 200 characters limit".into(),
        )));
    }
    let limit = query.limit.unwrap_or(10).clamp(1, 100);

    let results = state.geocoding_service.search_address(query_str, limit).await?;
    Ok(Json(results))
}

pub async fn realtime_sse_handler(
    State(state): State<Arc<AppState>>,
    Query(bbox): Query<BboxQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, HttpError> {
    let geo_bbox = GeoJsonBbox {
        min_lon: bbox.min_lon,
        min_lat: bbox.min_lat,
        max_lon: bbox.max_lon,
        max_lat: bbox.max_lat,
    };
    geo_bbox.validate()?;

    let rx = state.realtime_service.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(move |item| {
        if let Ok(hazard) = item {
            if RealtimeService::is_hazard_in_bbox(&hazard, &geo_bbox) {
                if let Ok(data) = serde_json::to_string(&hazard) {
                    return Some(Ok(Event::default().event("hazard").data(data)));
                }
            }
        }
        None
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
