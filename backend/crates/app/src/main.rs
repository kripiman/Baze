// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use auth::AuthService;
use baze_app::config::AppConfig;
use baze_app::rate_limit::RateLimiter;
use baze_app::router::{AppState, create_router};
use geocoding::PhotonGeocodingService;
use hazards::HazardService;
use realtime::RealtimeService;
use routing::ValhallaRoutingService;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "baze_server=info,baze_app=info,tower_http=info".into());
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let config =
        AppConfig::from_env().map_err(|e| format!("Configuration initialization failed: {}", e))?;

    tracing::info!(
        environment = %config.environment,
        host = config.host,
        port = config.port,
        "Starting Baze Backend Modular Monolith"
    );

    // Conexión perezosa a PostGIS (permite arranque sin requerir base activa para endpoints en memoria)
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect_lazy(&config.database_url)?;

    // Inicialización de servicios de dominio
    let realtime_service = RealtimeService::new(2048);
    let auth_service = AuthService::new(pool.clone(), config.jwt_secret.clone());
    let hazard_service = Arc::new(HazardService::new(
        pool.clone(),
        config.confirmation_threshold,
        config.default_ttl_hours,
        Arc::new(realtime_service.clone()),
    ));

    // Tarea periódica de purga de reportes expirados (AGENTS §4.4)
    let purge_service = hazard_service.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Ok(purged) = purge_service.purge_expired() {
                if purged > 0 {
                    tracing::debug!(purged_count = purged, "Purged expired hazard reports");
                }
            }
        }
    });

    // Wiring arquitectónico: Routing comparte la misma instancia Arc<HazardService> que implementa CorridorHazards
    let routing_service =
        ValhallaRoutingService::new(config.valhalla_url.clone(), hazard_service.clone());

    let geocoding_service = Arc::new(PhotonGeocodingService::new(config.photon_url.clone()));
    let rate_limiter = RateLimiter::new();

    let app_state = Arc::new(AppState {
        config: config.clone(),
        auth_service,
        hazard_service,
        routing_service,
        geocoding_service,
        realtime_service,
        rate_limiter,
    });

    let mut app = create_router(app_state).layer(TraceLayer::new_for_http());

    // En desarrollo, permitir CORS amplio para prototipado rápido y emuladores
    if config.is_development() {
        let cors = CorsLayer::new()
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
            .allow_origin(Any);
        app = app.layer(cors);
    }

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Baze API listening on http://{}", addr);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;

    Ok(())
}
