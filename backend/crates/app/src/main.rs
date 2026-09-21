// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use auth::AuthService;
use baze_app::config::AppConfig;
use baze_app::router::{create_router, AppState};
use geocoding::PhotonGeocodingService;
use hazards::HazardService;
use realtime::RealtimeService;
use routing::ValhallaRoutingService;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "baze_server=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = AppConfig::from_env().map_err(|e| format!("Configuration error: {}", e))?;
    tracing::info!(
        environment = %config.environment,
        host = %config.host,
        port = %config.port,
        "Starting Baze backend server"
    );

    // Conexión perezosa a la base de datos para que el esqueleto arranque aún sin PostGIS activo
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect_lazy(&config.database_url)?;

    // Inicialización de servicios de dominio
    let auth_service = AuthService::new(pool.clone(), config.jwt_secret.clone());
    let hazard_service = HazardService::new(
        pool.clone(),
        config.confirmation_threshold,
        config.default_ttl_hours,
    );
    let shared_hazards = Arc::new(hazard_service.clone());

    // Wiring arquitectónico: Routing depende de traits de shared implementados por hazards
    let routing_service = ValhallaRoutingService::new(
        config.valhalla_url.clone(),
        shared_hazards.clone(), // impl HazardBlockingReader
        shared_hazards,         // impl HazardReader
    );

    let geocoding_service = Arc::new(PhotonGeocodingService::new(config.photon_url.clone()));
    let realtime_service = RealtimeService::new(2048);
    let rate_limiter = baze_app::rate_limit::RateLimiter::new(config.rate_limit_rpm);

    let app_state = Arc::new(AppState {
        config: config.clone(),
        auth_service,
        hazard_service,
        routing_service,
        geocoding_service,
        realtime_service,
        rate_limiter,
    });

    // Configuración estricta de CORS según el entorno
    let cors = if config.is_development() {
        CorsLayer::new()
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
                axum::http::header::ACCEPT,
            ])
            .allow_origin(Any)
    } else {
        CorsLayer::new()
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::OPTIONS,
            ])
            .allow_headers([
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
                axum::http::header::ACCEPT,
            ])
    };

    let app = create_router(app_state)
        .layer(TraceLayer::new_for_http())
        .layer(cors);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Baze API listening on http://{}", addr);

    axum::serve(listener, app).await?;
    Ok(())
}
