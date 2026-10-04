// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use auth::AuthService;
use auth::token::TokenKeys;
use baze_app::config::AppConfig;
use baze_app::rate_limit::RateLimiter;
use baze_app::router::{AppState, build_app};
use baze_app::server::{ServerOptions, serve, shutdown_signal};
use geocoding::PhotonGeocodingService;
use hazards::{HazardPolicy, HazardService};
use realtime::RealtimeService;
use routing::ValhallaRoutingService;
use std::sync::Arc;
use std::time::Duration;

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

    // La base de datos es obligatoria: las cuentas viven en ella. Se espera a que esté lista, y se
    // rechazan los roles con privilegios de más y los esquemas sin migrar o alterados.
    let pool = baze_app::db::connect(&config.database_url, 20, Duration::from_secs(30)).await?;
    if !config.is_development() {
        baze_app::db::assert_least_privilege(&pool).await?;
    }
    baze_app::db::assert_schema_current(&pool).await?;

    // Inicialización de servicios de dominio
    let realtime_service = RealtimeService::new(2048);
    let auth_service = Arc::new(AuthService::new(
        pool.clone(),
        TokenKeys::new(
            config.jwt_secret.as_bytes(),
            config
                .jwt_secret_previous
                .as_deref()
                .map(|s| s.as_bytes().to_vec()),
        ),
        chrono::Duration::days(i64::from(config.auth_token_ttl_days)),
    ));
    let hazard_service = Arc::new(HazardService::new(
        pool.clone(),
        HazardPolicy {
            confirmation_threshold: config.confirmation_threshold,
            default_ttl_hours: config.default_ttl_hours,
            min_account_age: chrono::Duration::seconds(config.account_min_age_secs as i64),
            reports_per_day: i64::from(config.reports_per_account_per_day),
            votes_per_day: i64::from(config.votes_per_account_per_day),
        },
        // Keys the tags of voter networks (ADR-0008); a distinct label separates it from token MACs.
        config.jwt_secret.as_bytes(),
        Arc::new(realtime_service.clone()),
    ));

    // Tarea periódica de purga de reportes expirados (AGENTS §4.4). Las lecturas ya ignoran los
    // caducados; esto solo recupera espacio. Un DELETE interrumpido se revierte entero.
    let purge_service = hazard_service.clone();
    let purge_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            interval.tick().await;
            match purge_service.purge_expired().await {
                Ok(0) => {}
                Ok(purged) => tracing::info!(purged, "Purged expired hazard reports"),
                Err(error) => tracing::warn!(%error, "Purging expired hazard reports failed"),
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
    purge_task.abort();
    tracing::info!("Baze API stopped");

    Ok(())
}
