// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use chrono::{Duration, Utc};
use shared::{
    AppError, BoundingBox, CorridorHazards, CreateHazardRequest, GeoJsonLineString, GeoJsonPolygon,
    Hazard, HazardNotifier, HazardStatus, HazardType, IpNet, Vote,
};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

/// A listing never returns more than this many reports (newest first).
pub const MAX_LIST_RESULTS: usize = 500;

/// Representa la subred evaluada para la legitimidad del voto (/24 para IPv4, /64 para IPv6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct VoterNetwork(IpNet);

impl VoterNetwork {
    const IPV4_PREFIX: u8 = 24;
    const IPV6_PREFIX: u8 = 64;

    fn from_ip(ip: IpAddr) -> Self {
        Self(shared::network_of(ip, Self::IPV4_PREFIX, Self::IPV6_PREFIX))
    }
}

struct Ballot {
    vote: Vote,
    counts: bool,
}

struct HazardRecord {
    hazard: Hazard,
    ballots: HashMap<Uuid, Ballot>,
    claimed_subnets: HashSet<VoterNetwork>,
}

impl HazardRecord {
    fn new(hazard: Hazard) -> Self {
        Self {
            hazard,
            ballots: HashMap::new(),
            claimed_subnets: HashSet::new(),
        }
    }

    fn cast(
        &mut self,
        account: Uuid,
        vote: Vote,
        network: VoterNetwork,
        threshold: i32,
    ) -> &Hazard {
        let claimed = &mut self.claimed_subnets;
        self.ballots
            .entry(account)
            .or_insert_with(|| Ballot {
                vote,
                counts: claimed.insert(network),
            })
            .vote = vote;

        let (up, down) =
            self.ballots
                .values()
                .filter(|b| b.counts)
                .fold((0, 0), |(u, d), b| match b.vote {
                    Vote::Up => (u + 1, d),
                    Vote::Down => (u, d + 1),
                });

        self.hazard.upvotes = up;
        self.hazard.downvotes = down;
        self.hazard.status = evaluate_hazard_status(self.hazard.hazard_type, up, down, threshold);
        &self.hazard
    }
}

#[derive(Clone)]
pub struct HazardService {
    #[allow(dead_code)]
    pool: PgPool,
    confirmation_threshold: i32,
    default_ttl_hours: i64,
    records: Arc<RwLock<HashMap<Uuid, HazardRecord>>>,
    notifier: Arc<dyn HazardNotifier>,
}

/// Función pura para determinar el estado de confirmación de un reporte según sus votos y umbral.
pub fn evaluate_hazard_status(
    hazard_type: HazardType,
    upvotes: i32,
    downvotes: i32,
    confirmation_threshold: i32,
) -> HazardStatus {
    match hazard_type {
        HazardType::Warning => HazardStatus::Confirmed,
        HazardType::Blocking => {
            let balance = upvotes - downvotes;
            if balance >= confirmation_threshold {
                HazardStatus::Confirmed
            } else {
                HazardStatus::Unconfirmed
            }
        }
    }
}

impl HazardService {
    pub fn new(
        pool: PgPool,
        confirmation_threshold: i32,
        default_ttl_hours: i64,
        notifier: Arc<dyn HazardNotifier>,
    ) -> Self {
        Self {
            pool,
            confirmation_threshold,
            default_ttl_hours,
            records: Arc::new(RwLock::new(HashMap::new())),
            notifier,
        }
    }

    pub async fn create_hazard(
        &self,
        account_id: Uuid,
        req: &CreateHazardRequest,
        client_ip: IpAddr,
    ) -> Result<Hazard, AppError> {
        let now = Utc::now();
        let expires_at = now + Duration::hours(self.default_ttl_hours);
        let hazard_type = req.category.hazard_type();

        let initial_hazard = Hazard {
            id: Uuid::new_v4(),
            category: req.category,
            hazard_type,
            status: HazardStatus::Unconfirmed,
            description: req.description.clone(),
            upvotes: 0,
            downvotes: 0,
            location: req.location.clone(),
            created_at: now,
            expires_at,
        };

        let mut record = HazardRecord::new(initial_hazard);
        record.cast(
            account_id,
            Vote::Up,
            VoterNetwork::from_ip(client_ip),
            self.confirmation_threshold,
        );
        let hazard = record.hazard.clone();

        {
            let mut records = self
                .records
                .write()
                .map_err(|_| AppError::Internal("Lock poisoned".into()))?;
            records.insert(hazard.id, record);
        }

        // Emitir notificación fuera del lock
        self.notifier.broadcast_hazard(&hazard);

        Ok(hazard)
    }

    pub async fn vote_hazard(
        &self,
        hazard_id: Uuid,
        account_id: Uuid,
        vote: Vote,
        client_ip: IpAddr,
    ) -> Result<Hazard, AppError> {
        let now = Utc::now();

        let updated_hazard = {
            let mut records = self
                .records
                .write()
                .map_err(|_| AppError::Internal("Lock poisoned".into()))?;

            let record = records
                .get_mut(&hazard_id)
                .ok_or_else(|| AppError::NotFound(format!("Hazard {} not found", hazard_id)))?;

            // Validar expiración (devuelve NotFound)
            if record.hazard.expires_at <= now {
                return Err(AppError::NotFound(format!(
                    "Hazard {} has expired",
                    hazard_id
                )));
            }

            record.cast(
                account_id,
                vote,
                VoterNetwork::from_ip(client_ip),
                self.confirmation_threshold,
            );
            record.hazard.clone()
        };

        // Emitir notificación fuera del lock
        self.notifier.broadcast_hazard(&updated_hazard);

        Ok(updated_hazard)
    }

    pub async fn list_active_hazards(&self, bbox: &BoundingBox) -> Result<Vec<Hazard>, AppError> {
        let now = Utc::now();
        let records = self
            .records
            .read()
            .map_err(|_| AppError::Internal("Lock poisoned".into()))?;

        let mut active: Vec<Hazard> = records
            .values()
            .map(|r| &r.hazard)
            .filter(|h| h.expires_at > now && bbox.contains_point(&h.location))
            .cloned()
            .collect();
        active.sort_unstable_by_key(|h| std::cmp::Reverse(h.created_at));
        active.truncate(MAX_LIST_RESULTS);

        Ok(active)
    }

    /// Purga los registros de reportes caducados de memoria (AGENTS §4.4).
    /// Retorna la cantidad de incidentes eliminados.
    pub fn purge_expired(&self) -> Result<usize, AppError> {
        let now = Utc::now();
        let mut records = self
            .records
            .write()
            .map_err(|_| AppError::Internal("Lock poisoned".into()))?;

        let initial_count = records.len();
        records.retain(|_, record| record.hazard.expires_at > now);
        Ok(initial_count - records.len())
    }
}

#[async_trait]
impl CorridorHazards for HazardService {
    async fn find_blocking_polygons_along_corridor(
        &self,
        _corridor: &GeoJsonLineString,
        _buffer_meters: f64,
    ) -> Result<Vec<GeoJsonPolygon>, AppError> {
        // TODO(verify): Implementar consulta PostGIS ST_Intersects(geom, ST_Buffer(ST_GeomFromGeoJSON($1)::geography, $2)::geometry)
        // filtrando WHERE hazard_type = 'blocking' AND status = 'confirmed' AND expires_at > NOW()
        Ok(Vec::new())
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

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{GeoJsonPoint, HazardCategory};

    struct NoopNotifier;
    impl HazardNotifier for NoopNotifier {
        fn broadcast_hazard(&self, _hazard: &Hazard) {}
    }

    fn setup_service(threshold: i32) -> HazardService {
        let pool = PgPool::connect_lazy("postgres://localhost/dummy").unwrap();
        HazardService::new(pool, threshold, 24, Arc::new(NoopNotifier))
    }

    #[test]
    fn test_evaluate_hazard_status_pure() {
        assert_eq!(
            evaluate_hazard_status(HazardType::Warning, 1, 0, 3),
            HazardStatus::Confirmed
        );
        assert_eq!(
            evaluate_hazard_status(HazardType::Blocking, 1, 0, 3),
            HazardStatus::Unconfirmed
        );
        assert_eq!(
            evaluate_hazard_status(HazardType::Blocking, 3, 0, 3),
            HazardStatus::Confirmed
        );
        assert_eq!(
            evaluate_hazard_status(HazardType::Blocking, 3, 1, 3),
            HazardStatus::Unconfirmed
        );
        assert_eq!(
            evaluate_hazard_status(HazardType::Blocking, 1, 0, 1),
            HazardStatus::Confirmed
        );
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[tokio::test]
    async fn test_create_warning_is_confirmed() {
        let service = setup_service(3);
        let creator_id = Uuid::new_v4();
        let req = CreateHazardRequest {
            category: HazardCategory::Glass,
            description: None,
            location: GeoJsonPoint::new(-70.65, -33.45),
        };

        let hazard = service
            .create_hazard(creator_id, &req, ip("192.168.1.10"))
            .await
            .unwrap();
        assert_eq!(hazard.status, HazardStatus::Confirmed);
        assert_eq!(hazard.upvotes, 1);
        assert_eq!(hazard.downvotes, 0);
    }

    #[tokio::test]
    async fn test_create_blocking_is_unconfirmed_initially() {
        let service = setup_service(3);
        let creator_id = Uuid::new_v4();
        let req = CreateHazardRequest {
            category: HazardCategory::Construction,
            description: Some("Street works".into()),
            location: GeoJsonPoint::new(-70.65, -33.45),
        };

        let hazard = service
            .create_hazard(creator_id, &req, ip("192.168.1.10"))
            .await
            .unwrap();
        assert_eq!(hazard.status, HazardStatus::Unconfirmed);
        assert_eq!(hazard.upvotes, 1);
        assert_eq!(hazard.downvotes, 0);
    }

    #[tokio::test]
    async fn test_create_blocking_with_threshold_one_is_confirmed() {
        let service = setup_service(1);
        let creator_id = Uuid::new_v4();
        let req = CreateHazardRequest {
            category: HazardCategory::RoadClosed,
            description: None,
            location: GeoJsonPoint::new(-70.65, -33.45),
        };

        let hazard = service
            .create_hazard(creator_id, &req, ip("192.168.1.10"))
            .await
            .unwrap();
        assert_eq!(hazard.status, HazardStatus::Confirmed);
        assert_eq!(hazard.upvotes, 1);
    }

    #[tokio::test]
    async fn test_vote_blocking_hazard_confirmation_threshold_lifecycle() {
        let service = setup_service(3);
        let creator_id = Uuid::new_v4();
        let req = CreateHazardRequest {
            category: HazardCategory::RoadClosed,
            description: None,
            location: GeoJsonPoint::new(-70.65, -33.45),
        };

        let hazard = service
            .create_hazard(creator_id, &req, ip("192.168.1.10"))
            .await
            .unwrap();
        assert_eq!(hazard.status, HazardStatus::Unconfirmed);
        assert_eq!(hazard.upvotes, 1);

        // Voter 2 from subnet 2 upvotes: total effective upvotes = 2, net = 2 < 3 -> Unconfirmed
        let voter_2 = Uuid::new_v4();
        let updated = service
            .vote_hazard(hazard.id, voter_2, Vote::Up, ip("192.168.2.10"))
            .await
            .unwrap();
        assert_eq!(updated.upvotes, 2);
        assert_eq!(updated.downvotes, 0);
        assert_eq!(updated.status, HazardStatus::Unconfirmed);

        // Voter 3 from subnet 3 upvotes: total effective upvotes = 3, net = 3 >= 3 -> Confirmed!
        let voter_3 = Uuid::new_v4();
        let updated = service
            .vote_hazard(hazard.id, voter_3, Vote::Up, ip("192.168.3.10"))
            .await
            .unwrap();
        assert_eq!(updated.upvotes, 3);
        assert_eq!(updated.downvotes, 0);
        assert_eq!(updated.status, HazardStatus::Confirmed);

        // Voter 4 from subnet 4 downvotes: upvotes = 3, downvotes = 1, net = 2 < 3 -> Unconfirmed again
        let voter_4 = Uuid::new_v4();
        let updated = service
            .vote_hazard(hazard.id, voter_4, Vote::Down, ip("192.168.4.10"))
            .await
            .unwrap();
        assert_eq!(updated.upvotes, 3);
        assert_eq!(updated.downvotes, 1);
        assert_eq!(updated.status, HazardStatus::Unconfirmed);

        // Voter 4 changes vote to Up: upvotes = 4, downvotes = 0, net = 4 >= 3 -> Confirmed!
        let updated = service
            .vote_hazard(hazard.id, voter_4, Vote::Up, ip("192.168.4.10"))
            .await
            .unwrap();
        assert_eq!(updated.upvotes, 4);
        assert_eq!(updated.downvotes, 0);
        assert_eq!(updated.status, HazardStatus::Confirmed);
    }

    #[tokio::test]
    async fn test_sybil_multiple_accounts_same_subnet_cannot_confirm_hazard() {
        let service = setup_service(3);
        let creator_id = Uuid::new_v4();
        let req = CreateHazardRequest {
            category: HazardCategory::RoadClosed,
            description: None,
            location: GeoJsonPoint::new(-70.65, -33.45),
        };

        // Atacante crea reporte desde 192.168.1.10 (/24)
        let hazard = service
            .create_hazard(creator_id, &req, ip("192.168.1.10"))
            .await
            .unwrap();
        assert_eq!(hazard.status, HazardStatus::Unconfirmed);
        assert_eq!(hazard.upvotes, 1);

        // Atacante vota desde cuenta 2 en la misma subred /24 (192.168.1.50)
        let account_b = Uuid::new_v4();
        let v2 = service
            .vote_hazard(hazard.id, account_b, Vote::Up, ip("192.168.1.50"))
            .await
            .unwrap();
        assert_eq!(v2.upvotes, 1); // Deduplicado: no sube el conteo efectivo
        assert_eq!(v2.status, HazardStatus::Unconfirmed);

        // Atacante vota desde cuenta 3 en la misma subred /24 (192.168.1.99)
        let account_c = Uuid::new_v4();
        let v3 = service
            .vote_hazard(hazard.id, account_c, Vote::Up, ip("192.168.1.99"))
            .await
            .unwrap();
        assert_eq!(v3.upvotes, 1); // Permanece en 1
        assert_eq!(v3.status, HazardStatus::Unconfirmed);

        // Consenso real: votos desde redes externas independientes
        let external_1 = Uuid::new_v4();
        let v4 = service
            .vote_hazard(hazard.id, external_1, Vote::Up, ip("10.0.1.10"))
            .await
            .unwrap();
        assert_eq!(v4.upvotes, 2);
        assert_eq!(v4.status, HazardStatus::Unconfirmed);

        let external_2 = Uuid::new_v4();
        let v5 = service
            .vote_hazard(hazard.id, external_2, Vote::Up, ip("10.0.2.10"))
            .await
            .unwrap();
        assert_eq!(v5.upvotes, 3);
        assert_eq!(v5.status, HazardStatus::Confirmed); // Ahora sí se confirma legítimamente
    }

    #[tokio::test]
    async fn test_single_account_multiple_subnets_cannot_confirm_hazard() {
        let service = setup_service(3);
        let creator_id = Uuid::new_v4();
        let req = CreateHazardRequest {
            category: HazardCategory::RoadClosed,
            description: None,
            location: GeoJsonPoint::new(-70.65, -33.45),
        };

        // 1. Creador registra reporte desde subnet_a
        let hazard = service
            .create_hazard(creator_id, &req, ip("192.168.1.10"))
            .await
            .unwrap();
        assert_eq!(hazard.status, HazardStatus::Unconfirmed);
        assert_eq!(hazard.upvotes, 1);
        assert_eq!(hazard.downvotes, 0);

        // 2. Misma cuenta vota +1 rotando a subnet_b -> No debe sumar
        let v1 = service
            .vote_hazard(hazard.id, creator_id, Vote::Up, ip("192.168.2.10"))
            .await
            .unwrap();
        assert_eq!(v1.upvotes, 1);
        assert_eq!(v1.downvotes, 0);
        assert_eq!(v1.status, HazardStatus::Unconfirmed);

        // 3. Misma cuenta vota +1 rotando a subnet_c -> No debe sumar ni confirmar
        let v2 = service
            .vote_hazard(hazard.id, creator_id, Vote::Up, ip("192.168.3.10"))
            .await
            .unwrap();
        assert_eq!(v2.upvotes, 1);
        assert_eq!(v2.downvotes, 0);
        assert_eq!(v2.status, HazardStatus::Unconfirmed);

        // 4. Misma cuenta vota -1 desde subnet_d -> Actualiza su voto original en subnet_a a Down
        let v3 = service
            .vote_hazard(hazard.id, creator_id, Vote::Down, ip("192.168.4.10"))
            .await
            .unwrap();
        assert_eq!(v3.upvotes, 0);
        assert_eq!(v3.downvotes, 1);
        assert_eq!(v3.status, HazardStatus::Unconfirmed);

        // 5. Misma cuenta vuelve a votar +1 desde subnet_e -> Su voto original pasa a Up
        let v4 = service
            .vote_hazard(hazard.id, creator_id, Vote::Up, ip("192.168.5.10"))
            .await
            .unwrap();
        assert_eq!(v4.upvotes, 1);
        assert_eq!(v4.downvotes, 0);
        assert_eq!(v4.status, HazardStatus::Unconfirmed);

        // 6. Cuentas legítimas distintas en subnet_b y subnet_c confirman
        let user_b = Uuid::new_v4();
        let v_b = service
            .vote_hazard(hazard.id, user_b, Vote::Up, ip("192.168.2.10"))
            .await
            .unwrap();
        assert_eq!(v_b.upvotes, 2);

        let user_c = Uuid::new_v4();
        let v_c = service
            .vote_hazard(hazard.id, user_c, Vote::Up, ip("192.168.3.10"))
            .await
            .unwrap();
        assert_eq!(v_c.upvotes, 3);
        assert_eq!(v_c.status, HazardStatus::Confirmed);
    }

    #[tokio::test]
    async fn test_ipv6_mobile_64_subnets_isolation() {
        let service = setup_service(3);
        let creator_id = Uuid::new_v4();
        let req = CreateHazardRequest {
            category: HazardCategory::RoadClosed,
            description: None,
            location: GeoJsonPoint::new(-70.65, -33.45),
        };

        // Dispositivo móvil 1 en prefijo 2001:db8:85a3:0::/64
        let hazard = service
            .create_hazard(creator_id, &req, ip("2001:db8:85a3:0::1"))
            .await
            .unwrap();
        assert_eq!(hazard.status, HazardStatus::Unconfirmed);
        assert_eq!(hazard.upvotes, 1);

        // Otra cuenta en el mismo dispositivo o subred /64
        let user_same_64 = Uuid::new_v4();
        let v1 = service
            .vote_hazard(
                hazard.id,
                user_same_64,
                Vote::Up,
                ip("2001:db8:85a3:0:ffff::2"),
            )
            .await
            .unwrap();
        assert_eq!(v1.upvotes, 1); // Deduplicado dentro del mismo /64

        // Otro usuario móvil legítimo con su propio /64 (2001:db8:85a3:1::/64)
        let user_ext_1 = Uuid::new_v4();
        let v2 = service
            .vote_hazard(hazard.id, user_ext_1, Vote::Up, ip("2001:db8:85a3:1::1"))
            .await
            .unwrap();
        assert_eq!(v2.upvotes, 2);

        // Tercer usuario móvil con su propio /64 (2001:db8:85a3:2::/64)
        let user_ext_2 = Uuid::new_v4();
        let v3 = service
            .vote_hazard(hazard.id, user_ext_2, Vote::Up, ip("2001:db8:85a3:2::1"))
            .await
            .unwrap();
        assert_eq!(v3.upvotes, 3);
        assert_eq!(v3.status, HazardStatus::Confirmed);
    }

    #[tokio::test]
    async fn test_vote_idempotence() {
        let service = setup_service(3);
        let creator_id = Uuid::new_v4();
        let req = CreateHazardRequest {
            category: HazardCategory::Flood,
            description: None,
            location: GeoJsonPoint::new(-70.65, -33.45),
        };

        let hazard = service
            .create_hazard(creator_id, &req, ip("192.168.1.10"))
            .await
            .unwrap();
        let voter = Uuid::new_v4();

        let v1 = service
            .vote_hazard(hazard.id, voter, Vote::Up, ip("192.168.2.10"))
            .await
            .unwrap();
        assert_eq!(v1.upvotes, 2);

        // Votar exactamente lo mismo no incrementa de nuevo
        let v2 = service
            .vote_hazard(hazard.id, voter, Vote::Up, ip("192.168.2.10"))
            .await
            .unwrap();
        assert_eq!(v2.upvotes, 2);
        assert_eq!(v2.downvotes, 0);
    }

    #[tokio::test]
    async fn test_vote_not_found() {
        let service = setup_service(3);
        let non_existent_id = Uuid::new_v4();
        let voter = Uuid::new_v4();

        match service
            .vote_hazard(non_existent_id, voter, Vote::Up, ip("192.168.1.10"))
            .await
        {
            Err(AppError::NotFound(_)) => (),
            other => panic!("Expected NotFound, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_purge_expired_hazards() {
        let pool = PgPool::connect_lazy("postgres://localhost/dummy").unwrap();
        // default_ttl_hours = 0 para simular expiración inmediata
        let service = HazardService::new(pool, 3, 0, Arc::new(NoopNotifier));
        let creator_id = Uuid::new_v4();
        let req = CreateHazardRequest {
            category: HazardCategory::Glass,
            description: None,
            location: GeoJsonPoint::new(-70.65, -33.45),
        };

        service
            .create_hazard(creator_id, &req, ip("192.168.1.10"))
            .await
            .unwrap();
        // Al tener TTL = 0, expires_at ya pasó o es now
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let purged = service.purge_expired().unwrap();
        assert_eq!(purged, 1);

        let bbox = shared::BoundingBox {
            min_lon: -71.0,
            min_lat: -34.0,
            max_lon: -70.0,
            max_lat: -33.0,
        };
        let active = service.list_active_hazards(&bbox).await.unwrap();
        assert!(active.is_empty());
    }

    #[test]
    fn test_voter_network_prefixes() {
        let v4: IpAddr = "192.168.1.100".parse().unwrap();
        assert_eq!(VoterNetwork::from_ip(v4).0.to_string(), "192.168.1.0/24");

        let v6: IpAddr = "2001:db8:85a3:0:1234:8a2e:370:7334".parse().unwrap();
        assert_eq!(
            VoterNetwork::from_ip(v6).0.to_string(),
            "2001:db8:85a3::/64"
        );
    }

    #[tokio::test]
    async fn listing_is_capped_and_newest_first() {
        let service = setup_service(3);
        for _ in 0..(MAX_LIST_RESULTS + 100) {
            let req = CreateHazardRequest {
                category: HazardCategory::Pothole,
                description: None,
                location: GeoJsonPoint::new(-70.65, -33.45),
            };
            service
                .create_hazard(Uuid::new_v4(), &req, ip("192.168.1.10"))
                .await
                .unwrap();
        }
        let bbox = BoundingBox {
            min_lon: -70.7,
            min_lat: -33.5,
            max_lon: -70.6,
            max_lat: -33.4,
        };

        let listed = service.list_active_hazards(&bbox).await.unwrap();

        assert_eq!(listed.len(), MAX_LIST_RESULTS);
        assert!(
            listed
                .windows(2)
                .all(|w| w[0].created_at >= w[1].created_at)
        );
    }
}
