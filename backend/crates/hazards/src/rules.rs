// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The vote state machine, free of I/O so it can be checked exhaustively.

use shared::{HazardStatus, HazardType};

/// Función pura para determinar el estado de un reporte según sus votos contabilizados y el umbral.
///
/// - Si los votos en contra superan a los favorables por el umbral, la comunidad lo retira (`Resolved`),
///   sea aviso o bloqueo.
/// - Un aviso (`Warning`) nace confirmado: solo alerta, no afecta rutas.
/// - Un bloqueo (`Blocking`) necesita que los votos favorables superen a los contrarios por el umbral.
pub fn evaluate_hazard_status(
    hazard_type: HazardType,
    upvotes: i32,
    downvotes: i32,
    confirmation_threshold: i32,
) -> HazardStatus {
    let balance = upvotes - downvotes;
    if -balance >= confirmation_threshold {
        return HazardStatus::Resolved;
    }
    match hazard_type {
        HazardType::Warning => HazardStatus::Confirmed,
        HazardType::Blocking if balance >= confirmation_threshold => HazardStatus::Confirmed,
        HazardType::Blocking => HazardStatus::Unconfirmed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_evaluate_hazard_status_resolved_needs_the_threshold_against() {
        // Not enough against yet: net -2 with threshold 3.
        assert_eq!(
            evaluate_hazard_status(HazardType::Warning, 1, 3, 3),
            HazardStatus::Confirmed
        );
        assert_eq!(
            evaluate_hazard_status(HazardType::Blocking, 1, 3, 3),
            HazardStatus::Unconfirmed
        );
        // Net -3 retires either kind of report.
        assert_eq!(
            evaluate_hazard_status(HazardType::Warning, 0, 3, 3),
            HazardStatus::Resolved
        );
        assert_eq!(
            evaluate_hazard_status(HazardType::Blocking, 1, 4, 3),
            HazardStatus::Resolved
        );
    }

    #[test]
    fn test_evaluate_hazard_status_never_contradicts_itself() {
        for up in 0..30 {
            for down in 0..30 {
                for threshold in 2..8 {
                    let blocking =
                        evaluate_hazard_status(HazardType::Blocking, up, down, threshold);
                    let warning = evaluate_hazard_status(HazardType::Warning, up, down, threshold);
                    // Both kinds are retired by exactly the same evidence.
                    assert_eq!(
                        blocking == HazardStatus::Resolved,
                        warning == HazardStatus::Resolved
                    );
                    if blocking == HazardStatus::Confirmed {
                        assert!(up - down >= threshold);
                    }
                }
            }
        }
    }
}
