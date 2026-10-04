// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::config::AppConfig;
use crate::http::error::{AppJson, ErrorResponse, HttpError};
use crate::http::extract::{AuthenticatedAccount, ClientIp};
use crate::openapi::{HealthResponse, SourceResponse};
use crate::rate_limit::{RateLimiter, rate_limit_api_middleware, rate_limit_signup_middleware};
use auth::{AuthResponse, AuthService};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{Method, Request, StatusCode, header},
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use futures_util::stream::{Stream, StreamExt};
use hazards::HazardService;
use realtime::RealtimeService;
use routing::ValhallaRoutingService;
use shared::{
    BoundingBox, CreateHazardRequest, GeocodingItem, GeocodingProvider, GeocodingQuery, Hazard,
    HazardVoteRequest, MAX_LIST_BBOX_SPAN_DEGREES, MAX_STREAM_BBOX_SPAN_DEGREES, RouteRequest,
    RouteResponse,
};
use std::{sync::Arc, time::Duration};
use tower::limit::GlobalConcurrencyLimitLayer;
use tower_http::{
    cors::{Any, CorsLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
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

/// Largest request body any endpoint accepts. The biggest legitimate payload is a hazard report
/// (a point plus at most 500 characters of text), far below 2 KiB; the rest is slack.
pub const MAX_BODY_BYTES: usize = 16 * 1024;

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

    let mut router = open_routes.merge(auth_routes).merge(api_routes);

    // The contract is committed in contracts/openapi.json; the interactive docs are for development.
    if state.config.enable_api_docs {
        router = router.merge(
            SwaggerUi::new("/swagger-ui")
                .url("/api-docs/openapi.json", crate::openapi::ApiDoc::openapi()),
        );
    }

    router
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

/// The router wrapped with everything that protects the process: a per-request deadline, a global
/// cap on in-flight requests, request tracing that never records query strings (they carry
/// bounding boxes and search text) and, in development only, permissive CORS.
///
/// Layers listed last run first. The deadline and the cap apply until a response is produced, so
/// an SSE stream (whose response is produced immediately and then streams) is not cut by them.
pub fn build_app(state: Arc<AppState>) -> Router {
    let timeout = Duration::from_secs(state.config.request_timeout_secs);
    let max_in_flight = state.config.max_concurrent_requests;
    let development = state.config.is_development();

    let mut app = create_router(state)
        .layer(GlobalConcurrencyLimitLayer::new(max_in_flight))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            timeout,
        ))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                tracing::info_span!(
                    "http",
                    method = %request.method(),
                    path = %request.uri().path(),
                )
            }),
        );

    if development {
        app = app.layer(
            CorsLayer::new()
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT])
                .allow_origin(Any),
        );
    }
    app
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
        (status = 429, description = "Rate limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
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
        (status = 400, description = "Invalid bbox query", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
    )
)]
pub async fn list_hazards_handler(
    State(state): State<Arc<AppState>>,
    Query(bbox): Query<BoundingBox>,
) -> Result<AppJson<Vec<Hazard>>, HttpError> {
    bbox.validate()?;
    bbox.validate_max_span(MAX_LIST_BBOX_SPAN_DEGREES)?;
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
        (status = 400, description = "Validation failed", body = ErrorResponse),
        (status = 401, description = "Unauthorized caller", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Content-Type must be application/json", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
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
        (status = 400, description = "Invalid vote payload", body = ErrorResponse),
        (status = 401, description = "Unauthorized caller", body = ErrorResponse),
        (status = 404, description = "Hazard not found or expired", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Content-Type must be application/json", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal error", body = ErrorResponse)
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
        (status = 400, description = "Invalid coordinates", body = ErrorResponse),
        (status = 413, description = "Request body too large", body = ErrorResponse),
        (status = 415, description = "Content-Type must be application/json", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ErrorResponse),
        (status = 501, description = "Routing is not available yet", body = ErrorResponse),
        (status = 502, description = "Routing engine failed", body = ErrorResponse)
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
        (status = 400, description = "Missing or invalid query parameter", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ErrorResponse),
        (status = 501, description = "Address search is not available yet", body = ErrorResponse),
        (status = 502, description = "Geocoding engine failed", body = ErrorResponse)
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
        (status = 400, description = "Invalid bbox query", body = ErrorResponse),
        (status = 429, description = "Rate limit or concurrent connection limit exceeded", body = ErrorResponse)
    )
)]
pub async fn realtime_sse_handler(
    State(state): State<Arc<AppState>>,
    client_ip: ClientIp,
    Query(bbox): Query<BoundingBox>,
) -> Result<Sse<impl Stream<Item = Result<Event, axum::Error>>>, HttpError> {
    bbox.validate()?;
    bbox.validate_max_span(MAX_STREAM_BBOX_SPAN_DEGREES)?;

    let hazard_stream = state.realtime_service.stream_hazards(client_ip.0, bbox)?;

    let event_stream =
        hazard_stream.map(|hazard| Event::default().event("hazard").json_data(&*hazard));

    Ok(Sse::new(event_stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
