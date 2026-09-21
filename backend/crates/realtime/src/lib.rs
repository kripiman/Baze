// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

use async_trait::async_trait;
use shared::{AppError, GeoJsonBbox, Hazard, HazardNotifier};
use std::sync::Arc;
use tokio::sync::broadcast::{self, Receiver, Sender};

#[derive(Clone)]
pub struct RealtimeService {
    sender: Sender<Arc<Hazard>>,
}

impl RealtimeService {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn subscribe(&self) -> Receiver<Arc<Hazard>> {
        self.sender.subscribe()
    }

    pub fn is_hazard_in_bbox(hazard: &Hazard, bbox: &GeoJsonBbox) -> bool {
        let lon = hazard.location.lon();
        let lat = hazard.location.lat();
        lon >= bbox.min_lon && lon <= bbox.max_lon && lat >= bbox.min_lat && lat <= bbox.max_lat
    }
}

impl Default for RealtimeService {
    fn default() -> Self {
        Self::new(1024)
    }
}

#[async_trait]
impl HazardNotifier for RealtimeService {
    async fn broadcast_hazard(&self, hazard: &Hazard) -> Result<(), AppError> {
        let arc_hazard = Arc::new(hazard.clone());
        // Enviar a todos los suscriptores activos en memoria
        let _ = self.sender.send(arc_hazard);
        Ok(())
    }
}
