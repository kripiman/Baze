// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use auth::AuthService;
use baze_app::config::AppConfig;
use baze_app::rate_limit::RateLimiter;
use baze_app::router::{AppState, build_app};
use baze_app::server::{ServerOptions, serve, shutdown_signal};
use geocoding::PhotonGeocodingService;
use hazards::HazardService;
use realtime::RealtimeService;
use routing::ValhallaRoutingService;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1).as_deref() {
        // The container HEALTHCHECK; it must not start a server.
        Some("healthcheck") => std::process::exit(if baze_app::healthcheck::probe_local() {
            0
        } else {
            1
        }),
        // Applies the database migrations with the migration role and exits.
        Some("migrate") => migrate(),
        Some(other) => Err(format!(
            "unknown command '{other}': use `healthcheck`, `migrate`, or no argument to serve"
        )
        .into()),
        None => run(),
    }
}

#[tokio::main]
async fn migrate() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let applied = baze_app::db::migrate_from_env().await?;
    tracing::info!(applied, "Database migrations are up to date");
    Ok(())
}

#[tokio::main]
async fn run() -> Result<(), Box<dyn std::error::Error>> {
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
            if let Ok(purged) = purge_service.purge_expired()
                && purged > 0
            {
                tracing::debug!(purged_count = purged, "Purged expired hazard reports");
            }
        }
    });

    // Wiring arquitectónico: Routing comparte la misma instancia Arc<HazardService> que implementa CorridorHazards
    let routing_service =
        ValhallaRoutingService::new(config.valhalla_url.clone(), hazard_service.clone())?;

    let geocoding_service = Arc::new(PhotonGeocodingService::new(config.photon_url.clone())?);
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

    let app = build_app(app_state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Baze API listening on http://{}", addr);

    serve(listener, app, ServerOptions::default(), shutdown_signal()).await?;
    tracing::info!("Baze API stopped");

    Ok(())
}
