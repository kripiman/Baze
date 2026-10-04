// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use futures_util::StreamExt as FuturesStreamExt;
use futures_util::stream::Stream;
use shared::{AppError, BoundingBox, Hazard, HazardNotifier, IpNet};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast::{self, Receiver, Sender};
use tokio_stream::{StreamExt as TokioStreamExt, wrappers::BroadcastStream};

pub const DEFAULT_MAX_TOTAL_CONNECTIONS: usize = 8192;
pub const DEFAULT_MAX_CONNECTIONS_PER_KEY: usize = 25;
pub const DEFAULT_MAX_CONNECTION_DURATION: Duration = Duration::from_secs(1800);

struct SseConnectionGuard {
    tracker: Arc<Mutex<SseTracker>>,
    key: IpNet,
}

impl Drop for SseConnectionGuard {
    fn drop(&mut self) {
        if let Ok(mut tracker) = self.tracker.lock() {
            tracker.total_connections = tracker.total_connections.saturating_sub(1);
            if let Some(count) = tracker.connections_by_key.get_mut(&self.key) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    tracker.connections_by_key.remove(&self.key);
                }
            }
        }
    }
}

struct SseTracker {
    total_connections: usize,
    connections_by_key: HashMap<IpNet, usize>,
}

#[derive(Clone)]
pub struct RealtimeService {
    sender: Sender<Arc<Hazard>>,
    tracker: Arc<Mutex<SseTracker>>,
    max_total: usize,
    max_per_key: usize,
}

impl RealtimeService {
    pub fn new(capacity: usize) -> Self {
        Self::with_limits(
            capacity,
            DEFAULT_MAX_TOTAL_CONNECTIONS,
            DEFAULT_MAX_CONNECTIONS_PER_KEY,
        )
    }

    pub(crate) fn with_limits(capacity: usize, max_total: usize, max_per_key: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            tracker: Arc::new(Mutex::new(SseTracker {
                total_connections: 0,
                connections_by_key: HashMap::new(),
            })),
            max_total,
            max_per_key,
        }
    }

    /// Intenta adquirir un cupo de conexión SSE. Si la clave o el total supera el límite, retorna RateLimited.
    fn acquire_connection(&self, key: IpNet) -> Result<SseConnectionGuard, AppError> {
        let mut tracker = self
            .tracker
            .lock()
            .map_err(|_| AppError::Internal("Lock poisoned".into()))?;

        if tracker.total_connections >= self.max_total {
            return Err(AppError::RateLimited(
                "Global SSE concurrent connection limit reached. Please retry later.".into(),
            ));
        }

        let key_count = tracker.connections_by_key.entry(key).or_insert(0);
        if *key_count >= self.max_per_key {
            return Err(AppError::RateLimited(
                "Per-network SSE concurrent connection limit reached. Please close other connections.".into(),
            ));
        }

        *key_count += 1;
        tracker.total_connections += 1;
        Ok(SseConnectionGuard {
            tracker: self.tracker.clone(),
            key,
        })
    }

    fn subscribe(&self) -> Receiver<Arc<Hazard>> {
        self.sender.subscribe()
    }

    /// Crea un stream de eventos filtrado por BoundingBox y delimitado por la duración máxima de conexión (30 min).
    /// Adquiere y mantiene el cupo de conexión por red del cliente (/64 en IPv6, /32 en IPv4).
    pub fn stream_hazards(
        &self,
        client_ip: IpAddr,
        bbox: BoundingBox,
    ) -> Result<impl Stream<Item = Arc<Hazard>> + Send + 'static + use<>, AppError> {
        let key = shared::client_network(client_ip);
        let guard = self.acquire_connection(key)?;
        let rx = self.subscribe();

        let stream = TokioStreamExt::filter_map(BroadcastStream::new(rx), move |item| {
            let _keep_guard = &guard;
            match item {
                Ok(hazard) if bbox.contains_point(&hazard.location) => Some(hazard),
                _ => None,
            }
        });

        Ok(FuturesStreamExt::take_until(
            stream,
            tokio::time::sleep(DEFAULT_MAX_CONNECTION_DURATION),
        ))
    }
}

impl HazardNotifier for RealtimeService {
    fn broadcast_hazard(&self, hazard: &Hazard) {
        let arc_hazard = Arc::new(hazard.clone());
        let _ = self.sender.send(arc_hazard);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_connection_limits_and_drop() {
        let service = RealtimeService::with_limits(16, 50, 3);
        let key: IpNet = "192.168.1.10/32".parse().unwrap();

        let mut guards = Vec::new();
        for _ in 0..3 {
            guards.push(service.acquire_connection(key).unwrap());
        }

        // La 4ta conexión para la misma clave debe ser rechazada
        assert!(service.acquire_connection(key).is_err());

        // Diferente clave sí puede conectarse
        let key2: IpNet = "192.168.1.20/32".parse().unwrap();
        let guard2 = service.acquire_connection(key2);
        assert!(guard2.is_ok());

        // Al liberar una conexión, se vuelve a permitir
        guards.pop();
        assert!(service.acquire_connection(key).is_ok());
    }

    #[test]
    fn test_sse_global_connection_limit() {
        let service = RealtimeService::with_limits(16, 2, 2);
        let key1: IpNet = "192.168.1.1/32".parse().unwrap();
        let key2: IpNet = "192.168.1.2/32".parse().unwrap();
        let key3: IpNet = "192.168.1.3/32".parse().unwrap();

        let _g1 = service.acquire_connection(key1).unwrap();
        let _g2 = service.acquire_connection(key2).unwrap();

        // Tope global alcanzado
        assert!(service.acquire_connection(key3).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn test_stream_hazards_filtering_and_limits() {
        use chrono::Utc;
        use shared::{GeoJsonPoint, HazardCategory, HazardStatus};
        use uuid::Uuid;

        let service = RealtimeService::with_limits(16, 10, 1);
        let ip: IpAddr = "192.168.1.50".parse().unwrap();
        let bbox = BoundingBox {
            min_lon: -70.7,
            min_lat: -33.5,
            max_lon: -70.5,
            max_lat: -33.3,
        };

        let s1 = service.stream_hazards(ip, bbox.clone()).unwrap();
        assert!(service.stream_hazards(ip, bbox.clone()).is_err());
        drop(s1); // el cupo se libera al soltar el stream

        let mut s2 = Box::pin(service.stream_hazards(ip, bbox).unwrap());

        let fuera = Hazard {
            id: Uuid::new_v4(),
            category: HazardCategory::Glass,
            hazard_type: HazardCategory::Glass.hazard_type(),
            status: HazardStatus::Confirmed,
            description: None,
            upvotes: 1,
            downvotes: 0,
            location: GeoJsonPoint::new(-71.0, -33.4),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(24),
        };
        let dentro = Hazard {
            id: Uuid::new_v4(),
            category: HazardCategory::Pothole,
            hazard_type: HazardCategory::Pothole.hazard_type(),
            status: HazardStatus::Confirmed,
            description: None,
            upvotes: 1,
            downvotes: 0,
            location: GeoJsonPoint::new(-70.6, -33.4),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(24),
        };

        service.broadcast_hazard(&fuera);
        service.broadcast_hazard(&dentro);
        assert_eq!(FuturesStreamExt::next(&mut s2).await.unwrap().id, dentro.id); // filtro por bbox

        tokio::time::advance(DEFAULT_MAX_CONNECTION_DURATION + Duration::from_secs(1)).await;
        assert!(FuturesStreamExt::next(&mut s2).await.is_none()); // corte a los 30 min
    }
}
