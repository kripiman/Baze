// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Database plumbing: connecting, applying migrations and refusing to run in unsafe conditions.
//!
//! Two roles are involved (ADR-0008). The migration role owns the schema and is used only by
//! `baze-server migrate`. The application role (`baze_app`) can read and write rows but cannot
//! change the schema, create extensions or run programs on the database host, so a flaw in the
//! backend cannot become control of the database.

use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use std::time::{Duration, Instant};

/// Every migration in `backend/migrations`, embedded in the binary at compile time.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Connects, retrying until `wait` has elapsed: the database container may still be starting.
pub async fn connect(url: &str, max_connections: u32, wait: Duration) -> Result<PgPool, String> {
    let started = Instant::now();
    loop {
        let attempt = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url)
            .await;
        match attempt {
            Ok(pool) => return Ok(pool),
            Err(error) if started.elapsed() >= wait => {
                // The error text never contains the connection URL, so it is safe to report.
                return Err(format!("could not connect to the database: {error}"));
            }
            Err(error) => {
                tracing::info!(%error, "database not ready yet, retrying");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

/// Applies the pending migrations. Needs the migration role: it creates and alters tables.
pub async fn migrate(pool: &PgPool) -> Result<(), String> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|error| format!("migration failed: {error}"))
}

/// Connects with `DATABASE_URL` (the migration role) and applies the pending migrations.
/// This is what `baze-server migrate` runs.
pub async fn migrate_from_env() -> Result<usize, String> {
    let url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL must be set to run migrations".to_string())?;
    let pool = connect(&url, 2, Duration::from_secs(60)).await?;
    let before = applied_versions(&pool).await.unwrap_or_default().len();
    migrate(&pool).await?;
    let after = applied_versions(&pool).await?.len();
    pool.close().await;
    Ok(after.saturating_sub(before))
}

/// Refuses a role that could escape the application's sandbox. In production the backend must run
/// as the low-privilege application role, never as the owner of the database.
pub async fn assert_least_privilege(pool: &PgPool) -> Result<(), String> {
    let row = sqlx::query(
        "SELECT current_user::text AS name, rolsuper, rolcreaterole, rolcreatedb, rolreplication, rolbypassrls \
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| format!("could not inspect the database role: {error}"))?;

    let name: String = row.get("name");
    let excess: Vec<&str> = [
        ("SUPERUSER", "rolsuper"),
        ("CREATEROLE", "rolcreaterole"),
        ("CREATEDB", "rolcreatedb"),
        ("REPLICATION", "rolreplication"),
        ("BYPASSRLS", "rolbypassrls"),
    ]
    .into_iter()
    .filter(|(_, column)| row.get::<bool, _>(*column))
    .map(|(label, _)| label)
    .collect();

    if excess.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the database role '{name}' has excess privileges ({}); run the backend as a role without them (see docs/adr/0008)",
            excess.join(", ")
        ))
    }
}

async fn applied_versions(pool: &PgPool) -> Result<Vec<(i64, Vec<u8>)>, String> {
    sqlx::query("SELECT version, checksum FROM _sqlx_migrations WHERE success ORDER BY version")
        .fetch_all(pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|row| (row.get("version"), row.get("checksum")))
                .collect()
        })
        .map_err(|error| format!("could not read the applied migrations: {error}"))
}

/// Checks that every migration embedded in this binary has been applied and has not been altered
/// since. The application role cannot migrate, so a mismatch is reported instead of being "fixed".
pub async fn assert_schema_current(pool: &PgPool) -> Result<(), String> {
    let applied = applied_versions(pool).await.map_err(|error| {
        format!("{error}. Has `baze-server migrate` been run with the migration role?")
    })?;

    for migration in MIGRATOR
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
    {
        match applied
            .iter()
            .find(|(version, _)| *version == migration.version)
        {
            None => {
                return Err(format!(
                    "migration {} ({}) has not been applied; run `baze-server migrate` with the migration role",
                    migration.version, migration.description
                ));
            }
            Some((_, checksum)) if checksum.as_slice() != migration.checksum.as_ref() => {
                return Err(format!(
                    "migration {} ({}) differs from what was applied to the database; refusing to start",
                    migration.version, migration.description
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}
