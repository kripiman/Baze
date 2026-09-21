// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use chrono::{Duration, Utc};
use shared::{
    AppError, CreateHazardRequest, GeoJsonBbox, GeoJsonLineString, GeoJsonPolygon, Hazard,
    HazardBlockingReader, HazardReader, HazardStatus, HazardType, HazardWriter,
};
use sqlx::PgPool;
use uuid::Uuid;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct HazardService {
    #[allow(dead_code)]
    pool: PgPool,
    confirmation_threshold: i32,
    default_ttl_hours: i64,
    hazards: Arc<RwLock<HashMap<Uuid, Hazard>>>,
    votes: Arc<RwLock<HashMap<(Uuid, Uuid), i16>>>,
}

impl HazardService {
    pub fn new(pool: PgPool, confirmation_threshold: i32, default_ttl_hours: i64) -> Self {
        Self {
            pool,
            confirmation_threshold,
            default_ttl_hours,
            hazards: Arc::new(RwLock::new(HashMap::new())),
            votes: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl HazardBlockingReader for HazardService {
    async fn find_blocking_polygons_along_corridor(
        &self,
        _corridor: &GeoJsonLineString,
        _buffer_meters: f64,
    ) -> Result<Vec<GeoJsonPolygon>, AppError> {
        // TODO(verify): Implementar consulta PostGIS ST_Intersects(geom, ST_Buffer(ST_GeomFromGeoJSON($1)::geography, $2)::geometry)
        // filtrando WHERE hazard_type = 'blocking' AND status = 'confirmed' AND expires_at > NOW()
        // Convertir cada punto intersecado a un polígono buffer (ej. ST_AsGeoJSON(ST_Buffer(geom::geography, 15)::geometry))
        Ok(Vec::new())
    }
}

#[async_trait]
impl HazardReader for HazardService {
    async fn list_active_hazards(&self, bbox: &GeoJsonBbox) -> Result<Vec<Hazard>, AppError> {
        let now = Utc::now();
        let h_map = self
            .hazards
            .read()
            .map_err(|_| AppError::Internal("Lock poisoned".into()))?;
        let active: Vec<Hazard> = h_map
            .values()
            .filter(|h| h.expires_at > now && bbox.contains_point(&h.location))
            .cloned()
            .collect();
        Ok(active)
    }

    async fn list_hazards_near_corridor(
        &self,
        _corridor: &GeoJsonLineString,
        _buffer_meters: f64,
    ) -> Result<Vec<Hazard>, AppError> {
        // TODO(verify): Implementar consulta para obtener todos los peligros (warning y blocking) a lo largo de la ruta
        // WHERE ST_DWithin(geom::geography, ST_GeomFromGeoJSON($1)::geography, $2) AND expires_at > NOW()
        Ok(Vec::new())
    }
}

#[async_trait]
impl HazardWriter for HazardService {
    async fn create_hazard(
        &self,
        account_id: Uuid,
        req: &CreateHazardRequest,
    ) -> Result<Hazard, AppError> {
        let now = Utc::now();
        let expires_at = now + Duration::hours(self.default_ttl_hours);

        let initial_status = match req.hazard_type {
            HazardType::Warning => HazardStatus::Confirmed,
            HazardType::Blocking => HazardStatus::Unconfirmed, // Requiere confirmación por votos
        };

        let id = Uuid::new_v4();
        let hazard = Hazard {
            id,
            creator_account_id: account_id,
            category: req.category.clone(),
            hazard_type: req.hazard_type,
            status: initial_status,
            description: req.description.clone(),
            upvotes: 1,
            downvotes: 0,
            location: req.location.clone(),
            created_at: now,
            expires_at,
        };

        {
            let mut h_map = self
                .hazards
                .write()
                .map_err(|_| AppError::Internal("Lock poisoned".into()))?;
            let mut v_map = self
                .votes
                .write()
                .map_err(|_| AppError::Internal("Lock poisoned".into()))?;
            h_map.insert(id, hazard.clone());
            v_map.insert((id, account_id), 1); // Voto inicial del creador
        }

        Ok(hazard)
    }

    async fn vote_hazard(
        &self,
        hazard_id: Uuid,
        account_id: Uuid,
        vote_type: i16,
    ) -> Result<Hazard, AppError> {
        if vote_type != 1 && vote_type != -1 {
            return Err(AppError::Validation("Vote type must be 1 or -1".into()));
        }

        let mut h_map = self
            .hazards
            .write()
            .map_err(|_| AppError::Internal("Lock poisoned".into()))?;
        let mut v_map = self
            .votes
            .write()
            .map_err(|_| AppError::Internal("Lock poisoned".into()))?;

        let hazard = h_map
            .get_mut(&hazard_id)
            .ok_or_else(|| AppError::NotFound(format!("Hazard {} not found", hazard_id)))?;

        // Restricción anti-Sybil: un voto por cuenta por reporte (upsert)
        v_map.insert((hazard_id, account_id), vote_type);

        // Recalcular balance real de votos para el reporte
        let mut upvotes = 0;
        let mut downvotes = 0;
        for (&(h_id, _acc_id), &v) in v_map.iter() {
            if h_id == hazard_id {
                if v == 1 {
                    upvotes += 1;
                } else if v == -1 {
                    downvotes += 1;
                }
            }
        }

        hazard.upvotes = upvotes;
        hazard.downvotes = downvotes;

        // Regla de confirmación comunitaria (AGENTS.md):
        // - warning: siempre confirmado
        // - blocking: solo confirmado si balance (upvotes - downvotes) >= confirmation_threshold
        if hazard.hazard_type == HazardType::Blocking {
            let balance = upvotes - downvotes;
            if balance >= self.confirmation_threshold {
                hazard.status = HazardStatus::Confirmed;
            } else {
                hazard.status = HazardStatus::Unconfirmed;
            }
        }

        Ok(hazard.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::GeoJsonPoint;
    use sqlx::postgres::PgPoolOptions;

    fn setup_service(threshold: i32) -> HazardService {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://localhost/dummy")
            .unwrap();
        HazardService::new(pool, threshold, 24)
    }

    #[tokio::test]
    async fn test_create_warning_is_confirmed() {
        let service = setup_service(3);
        let creator_id = Uuid::new_v4();
        let req = CreateHazardRequest {
            category: "glass".into(),
            hazard_type: HazardType::Warning,
            description: None,
            location: GeoJsonPoint::new(-70.65, -33.45),
        };

        let hazard = service.create_hazard(creator_id, &req).await.unwrap();
        assert_eq!(hazard.status, HazardStatus::Confirmed);
        assert_eq!(hazard.upvotes, 1);
        assert_eq!(hazard.downvotes, 0);
    }

    #[tokio::test]
    async fn test_create_blocking_is_unconfirmed_initially() {
        let service = setup_service(3);
        let creator_id = Uuid::new_v4();
        let req = CreateHazardRequest {
            category: "construction".into(),
            hazard_type: HazardType::Blocking,
            description: Some("Street works".into()),
            location: GeoJsonPoint::new(-70.65, -33.45),
        };

        let hazard = service.create_hazard(creator_id, &req).await.unwrap();
        assert_eq!(hazard.status, HazardStatus::Unconfirmed);
        assert_eq!(hazard.upvotes, 1);
        assert_eq!(hazard.downvotes, 0);
    }

    #[tokio::test]
    async fn test_vote_blocking_hazard_confirmation_threshold_lifecycle() {
        let service = setup_service(3); // Threshold = 3
        let creator_id = Uuid::new_v4();
        let req = CreateHazardRequest {
            category: "road_closed".into(),
            hazard_type: HazardType::Blocking,
            description: None,
            location: GeoJsonPoint::new(-70.65, -33.45),
        };

        let hazard = service.create_hazard(creator_id, &req).await.unwrap();
        assert_eq!(hazard.status, HazardStatus::Unconfirmed);
        assert_eq!(hazard.upvotes, 1); // Creator's vote

        // Account 2 upvotes: total upvotes = 2, net balance = 2 < 3 -> Unconfirmed
        let voter_2 = Uuid::new_v4();
        let updated = service.vote_hazard(hazard.id, voter_2, 1).await.unwrap();
        assert_eq!(updated.upvotes, 2);
        assert_eq!(updated.downvotes, 0);
        assert_eq!(updated.status, HazardStatus::Unconfirmed);

        // Account 3 upvotes: total upvotes = 3, net balance = 3 >= 3 -> Confirmed!
        let voter_3 = Uuid::new_v4();
        let updated = service.vote_hazard(hazard.id, voter_3, 1).await.unwrap();
        assert_eq!(updated.upvotes, 3);
        assert_eq!(updated.downvotes, 0);
        assert_eq!(updated.status, HazardStatus::Confirmed);

        // Account 4 downvotes: upvotes = 3, downvotes = 1, net balance = 2 < 3 -> Unconfirmed again!
        let voter_4 = Uuid::new_v4();
        let updated = service.vote_hazard(hazard.id, voter_4, -1).await.unwrap();
        assert_eq!(updated.upvotes, 3);
        assert_eq!(updated.downvotes, 1);
        assert_eq!(updated.status, HazardStatus::Unconfirmed);

        // Account 4 changes vote to upvote: upvotes = 4, downvotes = 0, balance = 4 >= 3 -> Confirmed!
        let updated = service.vote_hazard(hazard.id, voter_4, 1).await.unwrap();
        assert_eq!(updated.upvotes, 4);
        assert_eq!(updated.downvotes, 0);
        assert_eq!(updated.status, HazardStatus::Confirmed);
    }

    #[tokio::test]
    async fn test_vote_invalid_type_and_not_found() {
        let service = setup_service(3);
        let non_existent_id = Uuid::new_v4();
        let voter = Uuid::new_v4();

        // Invalid vote type rejected
        assert!(service.vote_hazard(non_existent_id, voter, 0).await.is_err());
        assert!(service.vote_hazard(non_existent_id, voter, 2).await.is_err());

        // Valid vote on non-existent hazard returns NotFound
        match service.vote_hazard(non_existent_id, voter, 1).await {
            Err(AppError::NotFound(_)) => (),
            other => panic!("Expected NotFound, got {:?}", other),
        }
    }
}
