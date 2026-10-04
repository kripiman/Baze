// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Community hazard reports, persisted in PostGIS.
//!
//! The database is the only source of truth: nothing is cached in the process, so a restart loses
//! nothing and several instances can share one database. Votes on one report are serialised by a
//! row lock, which is what makes the "first vote per network counts" rule race-free.

mod rules;
mod voter_net;

#[cfg(all(test, feature = "db-tests"))]
mod store_tests;

pub use rules::evaluate_hazard_status;
pub use voter_net::{TAG_LEN, VoterNetKey};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use shared::{
    AccountContext, AppError, BoundingBox, CorridorHazards, CreateHazardRequest, GeoJsonLineString,
    GeoJsonPoint, GeoJsonPolygon, Hazard, HazardCategory, HazardNotifier, HazardStatus,
    HazardStore, HazardType, Vote,
};
use sqlx::{PgPool, Postgres, Transaction};
use std::net::IpAddr;
use std::sync::Arc;
use uuid::Uuid;
use voter_net::VoterNetwork;

/// A listing never returns more than this many reports (newest first).
pub const MAX_LIST_RESULTS: usize = 500;

/// Radius (metres) of the area a confirmed blocking report closes to routing.
pub const BLOCKING_EXCLUSION_RADIUS_METERS: f64 = 30.0;

/// Most points a corridor may have. The routing crate simplifies routes to fit, so this is shared.
pub use shared::MAX_CORRIDOR_POINTS;

/// Widest corridor (metres on each side of the route) a query accepts.
pub const MAX_CORRIDOR_BUFFER_METERS: f64 = 5_000.0;

/// Expired reports removed per statement, so one purge never runs into the statement timeout.
const PURGE_BATCH: i64 = 1_000;

/// Window of the daily caps.
const CAP_WINDOW_HOURS: i32 = 24;

/// How long an expired report is kept: the caps count stored rows, so they must outlive the window.
const PURGE_GRACE_HOURS: i32 = CAP_WINDOW_HOURS;

/// Longest a report can live; guards the TTL against nonsense values.
const MAX_TTL_HOURS: i64 = 24 * 365;

/// The columns of a report as the API shows it. The location leaves the database as GeoJSON.
const COLUMNS: &str = "id, category, hazard_type, status, description, upvotes, downvotes, \
                       ST_AsGeoJSON(geom)::text AS location, created_at, expires_at";

#[derive(sqlx::FromRow)]
struct HazardRow {
    id: Uuid,
    category: String,
    hazard_type: String,
    status: String,
    description: Option<String>,
    upvotes: i32,
    downvotes: i32,
    location: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl HazardRow {
    /// The CHECK constraints make an unknown value impossible, so failing here means the schema
    /// and the code disagree; that is a server error, not something to paper over.
    fn into_hazard(self) -> Result<Hazard, AppError> {
        let corrupt = |what: &str, value: &str| {
            AppError::Internal(format!("hazard {} has an unknown {what}: {value}", self.id))
        };
        Ok(Hazard {
            id: self.id,
            category: HazardCategory::from_db(&self.category)
                .ok_or_else(|| corrupt("category", &self.category))?,
            hazard_type: HazardType::from_db(&self.hazard_type)
                .ok_or_else(|| corrupt("type", &self.hazard_type))?,
            status: HazardStatus::from_db(&self.status)
                .ok_or_else(|| corrupt("status", &self.status))?,
            description: self.description,
            upvotes: self.upvotes,
            downvotes: self.downvotes,
            location: serde_json::from_str::<GeoJsonPoint>(&self.location)
                .map_err(|e| AppError::Internal(format!("hazard {} location: {e}", self.id)))?,
            created_at: self.created_at,
            expires_at: self.expires_at,
        })
    }
}

/// A failed statement. Running out of connections or hitting the statement timeout means the
/// database is overloaded: tell the client to come back later instead of reporting a bug.
fn store_error(error: sqlx::Error) -> AppError {
    let overloaded = match &error {
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => true,
        // 57014: query_canceled (statement_timeout of the application role).
        sqlx::Error::Database(db) => db.code().as_deref() == Some("57014"),
        _ => false,
    };
    if overloaded {
        AppError::Unavailable("The database is busy. Please retry later.".into())
    } else {
        AppError::Internal(format!("hazard store: {error}"))
    }
}

/// The rules a [`HazardService`] enforces. Everything here is configuration, none of it is a secret.
#[derive(Debug, Clone)]
pub struct HazardPolicy {
    /// Net votes needed to confirm a blocking report, or to retire any report.
    pub confirmation_threshold: i32,
    /// How long a report lives (kept within one hour and one year).
    pub default_ttl_hours: i64,
    /// An account younger than this cannot vote or report a road closure.
    pub min_account_age: Duration,
    /// New reports one account may file in 24 hours.
    pub reports_per_day: i64,
    /// Votes on other people's reports one account may cast in 24 hours.
    pub votes_per_day: i64,
}

impl Default for HazardPolicy {
    fn default() -> Self {
        Self {
            confirmation_threshold: 3,
            default_ttl_hours: 24,
            min_account_age: Duration::seconds(600),
            reports_per_day: 30,
            votes_per_day: 200,
        }
    }
}

#[derive(Clone)]
pub struct HazardService {
    pool: PgPool,
    confirmation_threshold: i32,
    default_ttl_hours: i32,
    min_account_age: Duration,
    reports_per_day: i64,
    votes_per_day: i64,
    net_key: VoterNetKey,
    notifier: Arc<dyn HazardNotifier>,
}

impl HazardService {
    /// `voter_net_secret` keys the tags of voter networks (see [`VoterNetKey`]); it is a secret of
    /// the server, never stored.
    pub fn new(
        pool: PgPool,
        policy: HazardPolicy,
        voter_net_secret: &[u8],
        notifier: Arc<dyn HazardNotifier>,
    ) -> Self {
        Self {
            pool,
            confirmation_threshold: policy.confirmation_threshold,
            default_ttl_hours: i32::try_from(policy.default_ttl_hours.clamp(1, MAX_TTL_HOURS))
                .expect("clamped to a year of hours"),
            min_account_age: policy.min_account_age,
            reports_per_day: policy.reports_per_day,
            votes_per_day: policy.votes_per_day,
            net_key: VoterNetKey::new(voter_net_secret),
            notifier,
        }
    }

    /// Accounts are free and anonymous, so a brand new one proves nothing. Letting it vote or close
    /// roads would let an attacker mint the three accounts a confirmation needs in one afternoon.
    fn require_established(&self, account: &AccountContext, action: &str) -> Result<(), AppError> {
        let age = Utc::now() - account.created_at;
        if age >= self.min_account_age {
            return Ok(());
        }
        let wait = (self.min_account_age - age).num_seconds().max(0) as u64;
        let minutes = wait.div_ceil(60).max(1);
        Err(AppError::Forbidden(format!(
            "This account is too new to {action}. Try again in {minutes} minute(s)."
        )))
    }

    /// Serialises the writes of one account until the transaction ends, so two concurrent requests
    /// cannot both pass the daily-cap check. The lock is always taken before any row lock, in every
    /// code path, which keeps lock order consistent (no deadlocks).
    async fn lock_account(
        tx: &mut Transaction<'_, Postgres>,
        account_id: Uuid,
    ) -> Result<(), AppError> {
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('baze:account:' || $1::text, 0))",
        )
        .bind(account_id)
        .execute(&mut **tx)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    /// Removes reports that expired more than [`PURGE_GRACE_HOURS`] ago (and, by cascade, their votes),
    /// and returns how many went. Reads already ignore expired rows, so this only reclaims space
    /// (AGENTS §4.4). The grace is the length of the daily-cap window: the caps count stored rows,
    /// so a row must outlive the window or a short TTL would hand the quota back early.
    pub async fn purge_expired(&self) -> Result<u64, AppError> {
        let mut total = 0;
        loop {
            let removed = sqlx::query(
                "DELETE FROM hazards WHERE id IN \
                 (SELECT id FROM hazards \
                  WHERE expires_at <= now() - make_interval(hours => $2) \
                  ORDER BY expires_at LIMIT $1)",
            )
            .bind(PURGE_BATCH)
            .bind(PURGE_GRACE_HOURS)
            .execute(&self.pool)
            .await
            .map_err(store_error)?
            .rows_affected();
            total += removed;
            if removed < PURGE_BATCH as u64 {
                return Ok(total);
            }
        }
    }

    /// Records one ballot and returns the report as it stands afterwards. The caller holds the
    /// report's row lock, so the recount sees every other ballot and nobody else writes meanwhile.
    async fn cast_ballot(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        hazard_id: Uuid,
        hazard_type: HazardType,
        account_id: Uuid,
        vote: Vote,
        tag: &[u8],
    ) -> Result<Hazard, AppError> {
        // A new ballot counts only if no counted ballot of the same network exists yet. An existing
        // one keeps the `counts` and `voter_net` it was born with: changing the IP later cannot
        // turn a ballot that did not count into one that does, nor the reverse.
        sqlx::query(
            "INSERT INTO hazard_votes (hazard_id, account_id, vote_type, counts, voter_net) \
             VALUES ($1, $2, $3, \
                     NOT EXISTS (SELECT 1 FROM hazard_votes \
                                 WHERE hazard_id = $1 AND voter_net = $4 AND counts), \
                     $4) \
             ON CONFLICT (hazard_id, account_id) DO UPDATE SET vote_type = EXCLUDED.vote_type",
        )
        .bind(hazard_id)
        .bind(account_id)
        .bind(vote as i16)
        .bind(tag)
        .execute(&mut **tx)
        .await
        .map_err(store_error)?;

        let (up, down): (i32, i32) = sqlx::query_as(
            "SELECT (count(*) FILTER (WHERE vote_type = 1))::int4, \
                    (count(*) FILTER (WHERE vote_type = -1))::int4 \
             FROM hazard_votes WHERE hazard_id = $1 AND counts",
        )
        .bind(hazard_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(store_error)?;

        let status = evaluate_hazard_status(hazard_type, up, down, self.confirmation_threshold);
        let row: HazardRow = sqlx::query_as(&format!(
            "UPDATE hazards SET upvotes = $2, downvotes = $3, status = $4 \
             WHERE id = $1 RETURNING {COLUMNS}"
        ))
        .bind(hazard_id)
        .bind(up)
        .bind(down)
        .bind(status.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(store_error)?;
        row.into_hazard()
    }

    fn corridor_geojson(
        corridor: &GeoJsonLineString,
        buffer_meters: f64,
    ) -> Result<String, AppError> {
        validate_corridor(corridor, buffer_meters)?;
        serde_json::to_string(corridor)
            .map_err(|e| AppError::Internal(format!("corridor encoding: {e}")))
    }
}

/// The corridor comes from the routing engine, but it reaches SQL, so it is checked like any input.
fn validate_corridor(corridor: &GeoJsonLineString, buffer_meters: f64) -> Result<(), AppError> {
    let invalid = |message: &str| Err(AppError::Validation(message.to_string()));
    if corridor.geom_type != "LineString" {
        return invalid("Corridor must be a GeoJSON LineString");
    }
    if corridor.coordinates.len() < 2 || corridor.coordinates.len() > MAX_CORRIDOR_POINTS {
        return invalid("Corridor must have between 2 and 10000 points");
    }
    let in_range = |[lon, lat]: &[f64; 2]| {
        lon.is_finite() && lat.is_finite() && lon.abs() <= 180.0 && lat.abs() <= 90.0
    };
    if !corridor.coordinates.iter().all(in_range) {
        return invalid("Corridor coordinates must be finite longitude/latitude values");
    }
    if !(buffer_meters.is_finite()
        && buffer_meters > 0.0
        && buffer_meters <= MAX_CORRIDOR_BUFFER_METERS)
    {
        return invalid("Corridor buffer must be between 0 and 5000 meters");
    }
    Ok(())
}

const CREATE_SQL: &str = "INSERT INTO hazards \
     (id, creator_account_id, category, hazard_type, status, description, upvotes, downvotes, \
      geom, created_at, expires_at) \
     VALUES ($1, $2, $3, $4, $5, $6, 1, 0, \
             ST_SetSRID(ST_GeomFromGeoJSON($7::text), 4326), \
             now(), now() + make_interval(hours => $8)) \
     RETURNING id, category, hazard_type, status, description, upvotes, downvotes, \
               ST_AsGeoJSON(geom)::text AS location, created_at, expires_at";

const LIST_SQL: &str = "SELECT id, category, hazard_type, status, description, upvotes, downvotes, \
     ST_AsGeoJSON(geom)::text AS location, created_at, expires_at \
     FROM hazards \
     WHERE expires_at > now() AND status <> 'resolved' \
       AND geom && ST_MakeEnvelope($1, $2, $3, $4, 4326) \
     ORDER BY created_at DESC LIMIT $5";

const CORRIDOR_BLOCKING_SQL: &str = "SELECT \
     ST_AsGeoJSON(ST_Buffer(geom::geography, $3, 4)::geometry)::text \
     FROM hazards \
     WHERE hazard_type = 'blocking' AND status = 'confirmed' AND expires_at > now() \
       AND ST_DWithin(geom::geography, ST_SetSRID(ST_GeomFromGeoJSON($1::text), 4326)::geography, $2) \
     ORDER BY created_at DESC LIMIT $4";

const CORRIDOR_LIST_SQL: &str = "SELECT id, category, hazard_type, status, description, upvotes, \
     downvotes, ST_AsGeoJSON(geom)::text AS location, created_at, expires_at \
     FROM hazards \
     WHERE expires_at > now() AND status <> 'resolved' \
       AND ST_DWithin(geom::geography, ST_SetSRID(ST_GeomFromGeoJSON($1::text), 4326)::geography, $2) \
     ORDER BY created_at DESC LIMIT $3";

#[async_trait]
impl HazardStore for HazardService {
    async fn create_hazard(
        &self,
        account: &AccountContext,
        req: &CreateHazardRequest,
        client_ip: IpAddr,
    ) -> Result<Hazard, AppError> {
        let hazard_type = req.category.hazard_type();
        // A warning only alerts; a blocking report can divert routes, so it needs an established account.
        if hazard_type == HazardType::Blocking {
            self.require_established(account, "report a road closure")?;
        }
        // The reporter's own vote is the first one and always counts: nobody else has voted yet.
        let status = evaluate_hazard_status(hazard_type, 1, 0, self.confirmation_threshold);
        let location = serde_json::to_string(&req.location)
            .map_err(|e| AppError::Internal(format!("location encoding: {e}")))?;
        let tag = self.net_key.tag(VoterNetwork::from_ip(client_ip));

        let mut tx = self.pool.begin().await.map_err(store_error)?;
        Self::lock_account(&mut tx, account.account_id).await?;
        let filed: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM hazards \
             WHERE creator_account_id = $1 AND created_at > now() - make_interval(hours => $2)",
        )
        .bind(account.account_id)
        .bind(CAP_WINDOW_HOURS)
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if filed >= self.reports_per_day {
            return Err(AppError::RateLimited(format!(
                "Daily limit of {} reports reached. Try again later.",
                self.reports_per_day
            )));
        }
        let row: HazardRow = sqlx::query_as(CREATE_SQL)
            .bind(Uuid::new_v4())
            .bind(account.account_id)
            .bind(req.category.as_str())
            .bind(hazard_type.as_str())
            .bind(status.as_str())
            .bind(req.sanitized_description())
            .bind(location)
            .bind(self.default_ttl_hours)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
        sqlx::query(
            "INSERT INTO hazard_votes (hazard_id, account_id, vote_type, counts, voter_net) \
             VALUES ($1, $2, 1, TRUE, $3)",
        )
        .bind(row.id)
        .bind(account.account_id)
        .bind(&tag[..])
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.commit().await.map_err(store_error)?;

        let hazard = row.into_hazard()?;
        self.notifier.broadcast_hazard(&hazard);
        Ok(hazard)
    }

    async fn vote_hazard(
        &self,
        hazard_id: Uuid,
        account: &AccountContext,
        vote: Vote,
        client_ip: IpAddr,
    ) -> Result<Hazard, AppError> {
        self.require_established(account, "vote")?;
        let tag = self.net_key.tag(VoterNetwork::from_ip(client_ip));
        let not_found = || AppError::NotFound(format!("Hazard {hazard_id} not found"));

        let mut tx = self.pool.begin().await.map_err(store_error)?;
        Self::lock_account(&mut tx, account.account_id).await?;
        // The lock serialises every vote on this report. An expired report no longer exists for
        // voters; neither does one the community has retired.
        let locked: Option<(String, String)> = sqlx::query_as(
            "SELECT hazard_type, status FROM hazards \
             WHERE id = $1 AND expires_at > now() FOR UPDATE",
        )
        .bind(hazard_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let (hazard_type, status) = locked.ok_or_else(not_found)?;
        let hazard_type = HazardType::from_db(&hazard_type)
            .ok_or_else(|| AppError::Internal(format!("hazard {hazard_id}: type {hazard_type}")))?;
        if HazardStatus::from_db(&status) == Some(HazardStatus::Resolved) {
            return Err(not_found());
        }

        // Changing a vote already cast adds nothing, so only a first ballot on someone else's
        // report spends the daily quota (the reporter's own ballot never does).
        let already_voted: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM hazard_votes WHERE hazard_id = $1 AND account_id = $2)",
        )
        .bind(hazard_id)
        .bind(account.account_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if !already_voted {
            let cast: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM hazard_votes v JOIN hazards h ON h.id = v.hazard_id \
                 WHERE v.account_id = $1 AND h.creator_account_id <> $1 \
                   AND v.created_at > now() - make_interval(hours => $2)",
            )
            .bind(account.account_id)
            .bind(CAP_WINDOW_HOURS)
            .fetch_one(&mut *tx)
            .await
            .map_err(store_error)?;
            if cast >= self.votes_per_day {
                return Err(AppError::RateLimited(format!(
                    "Daily limit of {} votes reached. Try again later.",
                    self.votes_per_day
                )));
            }
        }

        let hazard = self
            .cast_ballot(
                &mut tx,
                hazard_id,
                hazard_type,
                account.account_id,
                vote,
                &tag,
            )
            .await?;
        tx.commit().await.map_err(store_error)?;

        self.notifier.broadcast_hazard(&hazard);
        Ok(hazard)
    }

    async fn list_active_hazards(&self, bbox: &BoundingBox) -> Result<Vec<Hazard>, AppError> {
        let rows: Vec<HazardRow> = sqlx::query_as(LIST_SQL)
            .bind(bbox.min_lon)
            .bind(bbox.min_lat)
            .bind(bbox.max_lon)
            .bind(bbox.max_lat)
            .bind(MAX_LIST_RESULTS as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
        rows.into_iter().map(HazardRow::into_hazard).collect()
    }
}

#[async_trait]
impl CorridorHazards for HazardService {
    /// One polygon per confirmed, unexpired blocking report within `buffer_meters` of the route.
    /// More than [`MAX_LIST_RESULTS`] is an error rather than a silent truncation: a route that
    /// ignores a closure it was never told about is worse than no route.
    async fn find_blocking_polygons_along_corridor(
        &self,
        corridor: &GeoJsonLineString,
        buffer_meters: f64,
    ) -> Result<Vec<GeoJsonPolygon>, AppError> {
        let line = Self::corridor_geojson(corridor, buffer_meters)?;
        let shapes: Vec<String> = sqlx::query_scalar(CORRIDOR_BLOCKING_SQL)
            .bind(line)
            .bind(buffer_meters)
            .bind(BLOCKING_EXCLUSION_RADIUS_METERS)
            .bind(MAX_LIST_RESULTS as i64 + 1)
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
        if shapes.len() > MAX_LIST_RESULTS {
            return Err(AppError::Unavailable(
                "Too many blocking reports along this route to avoid them all".into(),
            ));
        }
        shapes
            .iter()
            .map(|shape| {
                serde_json::from_str::<GeoJsonPolygon>(shape)
                    .map_err(|e| AppError::Internal(format!("blocking polygon: {e}")))
            })
            .collect()
    }

    /// Active reports of either kind within `buffer_meters` of the route, newest first.
    async fn list_hazards_near_corridor(
        &self,
        corridor: &GeoJsonLineString,
        buffer_meters: f64,
    ) -> Result<Vec<Hazard>, AppError> {
        let line = Self::corridor_geojson(corridor, buffer_meters)?;
        let rows: Vec<HazardRow> = sqlx::query_as(CORRIDOR_LIST_SQL)
            .bind(line)
            .bind(buffer_meters)
            .bind(MAX_LIST_RESULTS as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?;
        rows.into_iter().map(HazardRow::into_hazard).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(points: Vec<[f64; 2]>) -> GeoJsonLineString {
        GeoJsonLineString {
            geom_type: "LineString".into(),
            coordinates: points,
        }
    }

    fn rejects(corridor: &GeoJsonLineString, buffer: f64) {
        assert!(
            matches!(
                validate_corridor(corridor, buffer),
                Err(AppError::Validation(_))
            ),
            "{corridor:?} / {buffer}"
        );
    }

    #[test]
    fn a_sane_corridor_is_accepted() {
        let corridor = line(vec![[-70.65, -33.45], [-70.6, -33.4]]);
        assert!(validate_corridor(&corridor, 50.0).is_ok());
        assert!(validate_corridor(&corridor, MAX_CORRIDOR_BUFFER_METERS).is_ok());
    }

    #[test]
    fn corridors_that_would_reach_sql_malformed_are_rejected() {
        let ok = vec![[-70.65, -33.45], [-70.6, -33.4]];
        rejects(&line(vec![]), 50.0);
        rejects(&line(vec![[-70.65, -33.45]]), 50.0);
        rejects(&line(vec![[-70.65, -33.45]; MAX_CORRIDOR_POINTS + 1]), 50.0);
        rejects(&line(vec![[f64::NAN, 0.0], [1.0, 1.0]]), 50.0);
        rejects(&line(vec![[181.0, 0.0], [1.0, 1.0]]), 50.0);
        rejects(&line(vec![[0.0, 91.0], [1.0, 1.0]]), 50.0);
        let mut polygon_like = line(ok.clone());
        polygon_like.geom_type = "Polygon".into();
        rejects(&polygon_like, 50.0);
        for buffer in [
            0.0,
            -1.0,
            f64::NAN,
            f64::INFINITY,
            MAX_CORRIDOR_BUFFER_METERS + 1.0,
        ] {
            rejects(&line(ok.clone()), buffer);
        }
    }

    #[tokio::test]
    async fn the_ttl_is_kept_within_sane_bounds() {
        struct Quiet;
        impl HazardNotifier for Quiet {
            fn broadcast_hazard(&self, _: &Hazard) {}
        }
        let pool = PgPool::connect_lazy("postgres://localhost/unused").unwrap();
        let ttl = |hours: i64| {
            HazardService::new(
                pool.clone(),
                HazardPolicy {
                    default_ttl_hours: hours,
                    ..HazardPolicy::default()
                },
                b"k",
                Arc::new(Quiet),
            )
            .default_ttl_hours
        };
        assert_eq!(ttl(0), 1);
        assert_eq!(ttl(-5), 1);
        assert_eq!(ttl(24), 24);
        assert_eq!(ttl(i64::MAX), 24 * 365);
    }
}
