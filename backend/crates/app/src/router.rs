// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::config::AppConfig;
use crate::http::error::{AppJson, HttpError};
use crate::http::extract::{AuthenticatedAccount, ClientIp};
use crate::openapi::{HealthResponse, SourceResponse};
use crate::rate_limit::{
    rate_limit_api_middleware, rate_limit_signup_middleware, RateLimiter,
};
use auth::{AuthResponse, AuthService};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
    Json, Router,
};
use futures_util::stream::{Stream, StreamExt};
use hazards::HazardService;
use realtime::RealtimeService;
use routing::ValhallaRoutingService;
use shared::{
    BoundingBox, CreateHazardRequest, GeocodingItem, GeocodingProvider, GeocodingQuery, Hazard,
    HazardVoteRequest, RouteRequest, RouteResponse,
};
use std::{sync::Arc, time::Duration};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;
use uuid::Uuid;

pub struct AppState {
    pub config: AppConfig,
    pub auth_service: AuthService,
    pub hazard_service: Arc<HazardService>,
    pub routing_service: ValhallaRoutingService,
    pub geocoding_service: Arc<dyn GeocodingProvider>,
    pub realtime_service: RealtimeService,
    pub rate_limiter: RateLimiter,
}

pub fn create_router(state: Arc<AppState>) -> Router {
    let open_routes = Router::new()
        .route("/health", get(health_handler))
        .route("/source", get(source_handler));

    let auth_routes = Router::new()
        .route("/api/v1/auth/anonymous", post(anonymous_auth_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            rate_limit_signup_middleware,
        ));

    let api_routes = Router::new()
        .route(
            "/api/v1/hazards",
            get(list_hazards_handler).post(create_hazard_handler),
        )
        .route("/api/v1/hazards/{id}/vote", post(vote_hazard_handler))
        .route("/api/v1/routing/route", post(route_handler))
        .route("/api/v1/geocoding/search", get(geocoding_handler))
        .route("/api/v1/realtime/sse", get(realtime_sse_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            rate_limit_api_middleware,
        ));

    open_routes
        .merge(auth_routes)
        .merge(api_routes)
        .merge(
            SwaggerUi::new("/swagger-ui")
                .url("/api-docs/openapi.json", crate::openapi::ApiDoc::openapi()),
        )
        .with_state(state)
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    responses(
        (status = 200, description = "System is healthy", body = HealthResponse)
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
        (status = 200, description = "AGPLv3 source code repository and deployed commit information", body = SourceResponse)
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
        (status = 429, description = "Rate limit exceeded")
    )
)]
pub async fn anonymous_auth_handler(
    State(state): State<Arc<AppState>>,
) -> Result<(StatusCode, Json<AuthResponse>), HttpError> {
    let auth = state.auth_service.create_anonymous_account().await?;
    Ok((StatusCode::CREATED, Json(auth)))
}

#[utoipa::path(
    get,
    path = "/api/v1/hazards",
    tag = "hazards",
    params(BoundingBox),
    responses(
        (status = 200, description = "Active hazards within bounding box", body = Vec<Hazard>),
        (status = 400, description = "Invalid bbox query"),
        (status = 429, description = "Rate limit exceeded")
    )
)]
pub async fn list_hazards_handler(
    State(state): State<Arc<AppState>>,
    Query(bbox): Query<BoundingBox>,
) -> Result<AppJson<Vec<Hazard>>, HttpError> {
    bbox.validate()?;
    let hazards = state.hazard_service.list_active_hazards(&bbox).await?;
    Ok(AppJson(hazards))
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
    client_ip: ClientIp,
    AuthenticatedAccount(account_id): AuthenticatedAccount,
    AppJson(payload): AppJson<CreateHazardRequest>,
) -> Result<(StatusCode, AppJson<Hazard>), HttpError> {
    payload.validate()?;
    let hazard = state
        .hazard_service
        .create_hazard(account_id, &payload, client_ip.0)
        .await?;
    Ok((StatusCode::CREATED, AppJson(hazard)))
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
    client_ip: ClientIp,
    AuthenticatedAccount(account_id): AuthenticatedAccount,
    AppJson(payload): AppJson<HazardVoteRequest>,
) -> Result<AppJson<Hazard>, HttpError> {
    let updated = state
        .hazard_service
        .vote_hazard(id, account_id, payload.vote, client_ip.0)
        .await?;
    Ok(AppJson(updated))
}

#[utoipa::path(
    post,
    path = "/api/v1/routing/route",
    tag = "routing",
    request_body = RouteRequest,
    responses(
        (status = 200, description = "Calculated bicycle route", body = RouteResponse),
        (status = 400, description = "Invalid coordinates"),
        (status = 429, description = "Rate limit exceeded")
    )
)]
pub async fn route_handler(
    State(state): State<Arc<AppState>>,
    AppJson(payload): AppJson<RouteRequest>,
) -> Result<AppJson<RouteResponse>, HttpError> {
    payload.validate()?;
    let route = state.routing_service.route_bicycle(&payload).await?;
    Ok(AppJson(route))
}

#[utoipa::path(
    get,
    path = "/api/v1/geocoding/search",
    tag = "geocoding",
    params(GeocodingQuery),
    responses(
        (status = 200, description = "Geocoding suggestions", body = Vec<GeocodingItem>),
        (status = 400, description = "Missing or invalid query parameter"),
        (status = 429, description = "Rate limit exceeded")
    )
)]
pub async fn geocoding_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GeocodingQuery>,
) -> Result<AppJson<Vec<GeocodingItem>>, HttpError> {
    query.validate()?;
    let results = state
        .geocoding_service
        .search_address(query.sanitized_query(), query.effective_limit())
        .await?;
    Ok(AppJson(results))
}

#[utoipa::path(
    get,
    path = "/api/v1/realtime/sse",
    tag = "realtime",
    params(BoundingBox),
    responses(
        (status = 200, description = "Server-sent events stream of hazards within bounding box", content_type = "text/event-stream"),
        (status = 400, description = "Invalid bbox query"),
        (status = 429, description = "Rate limit or concurrent connection limit exceeded")
    )
)]
pub async fn realtime_sse_handler(
    State(state): State<Arc<AppState>>,
    client_ip: ClientIp,
    Query(bbox): Query<BoundingBox>,
) -> Result<Sse<impl Stream<Item = Result<Event, axum::Error>>>, HttpError> {
    bbox.validate()?;

    let hazard_stream = state
        .realtime_service
        .stream_hazards(client_ip.0, bbox)?;

    let event_stream = hazard_stream.map(|hazard| {
        Event::default().event("hazard").json_data(&*hazard)
    });

    Ok(Sse::new(event_stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
